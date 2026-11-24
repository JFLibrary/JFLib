// jumps -- (JD21, CN-06) line jumps: a drawing of crossing dynamic connectors built as
// JFDiagrammer builds it, one page per jump style, per LineJumpCode, per direction, a size, close
// crossings, slanted straight connectors and connectors with settings of their own; the jumps
// in the display list, the cells written and read back, the SVG and PNG export
//
//   check(h)          group "jumps": every page's jumps (count per connector, style, side,
//                     width) against what the page asks for; written and read back: the page's
//                     and the connectors' line jump cells and their Geometry rows unchanged (the
//                     jumps are no geometry), the same jumps after reading; the SVG export holds
//                     them (cubic pieces, gaps), the PNG export shows an arc above the line
//   write(dir)        --write-jd21-jumps DIR: JD21-Gegenprobe-Linienuebergaenge.vsdx and a PNG
//                     of each page drawn by JFDiagrammer's drawing code, to lay next to Visio
//
// The pages: three horizontal connectors (y 2, 4, 6 in, x 1 .. 9) and three vertical ones (x 3,
// 5, 7, y 1 .. 7), drawn in that order (the verticals lie above and have the higher IDs), each
// with free ends; a title box above them says what Visio should show.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::render::{self, Jump, JumpStyle, Options};
use diagram_core::{glue, CellAddr, DocKind, Document, Owner, PageRef, ShapeRef, SheetRef, Text, Value};
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
    /// the middle horizontal brought to the front
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
    Spec { name: "Standard", cells: &[], layout: Layout::Grid, title: "Ohne Linienübergangs-Zellen der Seite: Visios Vorgabe (JFDiagrammer: waagerechte springen, Bogen nach oben)", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, 1.0])) },
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
    Spec { name: "Wo zuletzt verlegt", cells: &[(CODE, 3.0)], layout: Layout::MiddleOnTop, title: "LineJumpCode 3: zuletzt verlegte (die senkrechten sind neuer; die mittlere waagerechte liegt oben)", want: ([0; 3], [3; 3]), look: None },
    Spec { name: "Wo zuletzt angezeigt", cells: &[(CODE, 4.0)], layout: Layout::MiddleOnTop, title: "LineJumpCode 4: die obere springt (die mittlere waagerechte liegt ganz oben)", want: ([0, 3, 0], [2; 3]), look: None },
    Spec { name: "Wo zuerst angezeigt", cells: &[(CODE, 5.0)], layout: Layout::MiddleOnTop, title: "LineJumpCode 5: die untere springt (die mittlere waagerechte liegt ganz oben)", want: ([3, 0, 3], [1; 3]), look: None },
    Spec { name: "Richtung unten", cells: &[(CODE, 1.0), ("PageLineJumpDirX", 2.0)], layout: Layout::Grid, title: "PageLineJumpDirX 2: waagerechte springen nach unten", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, -1.0])) },
    Spec { name: "Richtung rechts", cells: &[(CODE, 2.0), ("PageLineJumpDirY", 2.0)], layout: Layout::Grid, title: "PageLineJumpDirY 2: senkrechte springen nach rechts", want: ([0; 3], [3; 3]), look: Some((JumpStyle::Arc, [1.0, 0.0])) },
    Spec { name: "Richtung links", cells: &[(CODE, 2.0), ("PageLineJumpDirY", 1.0)], layout: Layout::Grid, title: "PageLineJumpDirY 1: senkrechte springen nach links", want: ([0; 3], [3; 3]), look: Some((JumpStyle::Arc, [-1.0, 0.0])) },
    Spec { name: "Größe", cells: &[(CODE, 1.0), ("LineJumpFactorX", 3.0), ("LineJumpFactorY", 0.3)], layout: Layout::Grid, title: "LineJumpFactorX 3 (waagerecht groß), LineToLineX 0,125 in", want: ([3; 3], [0; 3]), look: Some((JumpStyle::Arc, [0.0, 1.0])) },
    Spec { name: "Nah", cells: &[(CODE, 1.0)], layout: Layout::Close, title: "Zwei senkrechte 1,3 mm nebeneinander: ein breiter Sprung (angenommen)", want: ([2; 3], [0; 3]), look: None },
    Spec { name: "Schräg", cells: &[(CODE, 1.0)], layout: Layout::Slanted, title: "Gerade Verbinder schräg: die flachere springt, beim X die obere (angenommen)", want: ([0; 3], [0; 3]), look: None },
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
        match spec.layout {
            Layout::MiddleOnTop => tx.bring_to_front(hs[1]).map_err(err)?,
            Layout::Always => set(&mut tx, SheetRef::Shape(hs[2]), "ConLineJumpCode", 2.0)?,
            Layout::Own => {
                set(&mut tx, SheetRef::Shape(hs[2]), "ConLineJumpCode", 1.0)?;
                set(&mut tx, SheetRef::Shape(hs[1]), "ConLineJumpStyle", 3.0)?;
                set(&mut tx, SheetRef::Shape(hs[1]), "ConLineJumpDirX", 2.0)?;
                set(&mut tx, SheetRef::Shape(vs[1]), "ConLineJumpCode", 4.0)?;
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

fn jumps_of(d: &render::Drawing, s: ShapeRef) -> Vec<Jump> {
    d.jumps().find(|(c, _)| *c == s).map(|(_, j)| j.to_vec()).unwrap_or_default()
}

fn draw(doc: &Document, page: PageRef) -> render::Drawing {
    render::build(doc, page, &mut render::ApproxMeasure, &Options::default())
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
        let d = draw(&doc, b.page);
        let hs: Vec<usize> = b.hs.iter().map(|&s| jumps_of(&d, s).len()).collect();
        let vs: Vec<usize> = b.vs.iter().map(|&s| jumps_of(&d, s).len()).collect();
        if spec.layout == Layout::Slanted {
            let n: Vec<usize> = b.slanted.iter().map(|&s| jumps_of(&d, s).len()).collect();
            h.check("jumps: slanted straight connectors (flat one twice, the X's upper once)", n == [0, 1, 2], format!("{n:?}"));
            continue;
        }
        h.check(&format!("jumps: page {} counts", spec.name), hs == spec.want.0 && vs == spec.want.1, format!("{hs:?} {vs:?}"));
        if let Some((style, side)) = spec.look {
            let all: Vec<Jump> = b.hs.iter().chain(&b.vs).flat_map(|&s| jumps_of(&d, s)).collect();
            let ok = !all.is_empty() && all.iter().all(|j| j.style == style && (j.normal[0] - side[0]).abs() < 1e-9 && (j.normal[1] - side[1]).abs() < 1e-9);
            h.check(&format!("jumps: page {} style and side", spec.name), ok, format!("{:?}", all.first().map(|j| (j.style, j.normal))));
        }
        if spec.name == "Größe" {
            let j = jumps_of(&d, b.hs[0]);
            let w = j.first().map_or(0.0, |j| j.to - j.from);
            h.check("jumps: size LineJumpFactorX 3 x LineToLineX 0.125", (w - 0.375).abs() < 1e-9, format!("{w}"));
        }
        if spec.name == "Nah" {
            let j = jumps_of(&d, b.hs[0]);
            let w = j.first().map_or(0.0, |j| j.to - j.from);
            h.check("jumps: close crossings merged", (w - (0.05 + 0.125 * 2.0 / 3.0)).abs() < 1e-9, format!("{w}"));
        }
        if spec.name == "Eigene nie Stil" {
            let j = jumps_of(&d, b.hs[1]);
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
        let (d0, d1) = (draw(&doc, b.page), draw(&back, pb));
        for &s in b.hs.iter().chain(&b.vs).chain(&b.slanted) {
            let id = doc.shape(s).map_or(0, |x| x.id());
            let Some(t) = back.shape_by_id(Owner::Page(pb), id) else {
                cells_ok = false;
                continue;
            };
            for c in con_cells {
                cells_ok &= num(&doc, SheetRef::Shape(s), c) == num(&back, SheetRef::Shape(t), c);
            }
            rows_ok &= geometry_rows(&doc, s) == geometry_rows(&back, t) && geometry_rows(&back, t) <= 4;
            let (a, z) = (jumps_of(&d0, s), jumps_of(&d1, t));
            n += a.len();
            same_jumps &= a.len() == z.len() && a.iter().zip(&z).all(|(x, y)| (x.from - y.from).abs() < 1e-9 && x.style == y.style);
        }
    }
    h.check("jumps: page and connector line jump cells read back", cells_ok, String::new());
    h.check("jumps: the connectors' Geometry rows unchanged (jumps are no geometry)", rows_ok, String::new());
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
