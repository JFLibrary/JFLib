// drawings -- the self-built drawings of the viewer tests (JD4): every row type, transforms,
// lines, fills, texts, pictures, pages and themes, written with the package builder (pkg.rs)
//
//   all() -> Vec<(name, Pkg)>     the drawings; each foreground page becomes a golden image
//                                  (tests/golden/<name>-<page>.png)
//
// Generic content only: shapes made up for these tests, nothing taken from a Visio file.  The
// document's "No Style" holds the defaults (0.75 pt black lines, white fill, Calibri 12 pt
// centred), so every shape names style 0 and sets only what it tests.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::pkg::{Pkg, CT_DRAWING, CT_PAGE, CT_PAGES, CT_THEME, DECL, NS};

/// the drawings: geometry, transform, lines, fills, text, pictures, pages
pub fn all() -> Vec<(&'static str, Pkg)> {
    alloc::vec![("geometry", geometry()), ("transform", transform()), ("lines", lines()), ("fills", fills()), ("text", text()), ("pictures", pictures()), ("pages", pages())]
}

const STYLES: &str = "<StyleSheets><StyleSheet ID='0' NameU='No Style' Name='No Style'>\
<Cell N='LineWeight' V='0.01041666666666667' U='PT'/><Cell N='LineColor' V='#000000'/><Cell N='LinePattern' V='1'/>\
<Cell N='Rounding' V='0'/><Cell N='LineCap' V='0'/><Cell N='LineColorTrans' V='0'/><Cell N='BeginArrow' V='0'/><Cell N='EndArrow' V='0'/>\
<Cell N='BeginArrowSize' V='2'/><Cell N='EndArrowSize' V='2'/>\
<Cell N='FillForegnd' V='#ffffff'/><Cell N='FillForegndTrans' V='0'/><Cell N='FillBkgnd' V='#ffffff'/><Cell N='FillBkgndTrans' V='0'/>\
<Cell N='FillPattern' V='1'/><Cell N='ShdwPattern' V='0'/>\
<Cell N='LeftMargin' V='0.05555555555555555' U='PT'/><Cell N='RightMargin' V='0.05555555555555555' U='PT'/>\
<Cell N='TopMargin' V='0.05555555555555555' U='PT'/><Cell N='BottomMargin' V='0.05555555555555555' U='PT'/>\
<Cell N='VerticalAlign' V='1'/><Cell N='TextBkgnd' V='0'/><Cell N='DefaultTabStop' V='0.5'/><Cell N='TextDirection' V='0'/>\
<Section N='Character'><Row IX='0'><Cell N='Font' V='Calibri'/><Cell N='Color' V='#000000'/><Cell N='Style' V='0'/>\
<Cell N='Size' V='0.1666666666666667' U='PT'/><Cell N='FontScale' V='1'/><Cell N='Letterspace' V='0'/></Row></Section>\
<Section N='Paragraph'><Row IX='0'><Cell N='IndFirst' V='0'/><Cell N='IndLeft' V='0'/><Cell N='IndRight' V='0'/>\
<Cell N='SpLine' V='-1.2' U='PER'/><Cell N='SpBefore' V='0'/><Cell N='SpAfter' V='0'/><Cell N='HorzAlign' V='1'/><Cell N='Bullet' V='0'/></Row></Section>\
</StyleSheet></StyleSheets>";

/// a drawing of pages: (name, page sheet cells and sections, shapes, background page id);
/// `extra` parts are added as given (rels from the page parts: (page index, id, type, target))
struct Doc {
    pages: Vec<PageDef>,
    theme: Option<String>,
    parts: Vec<(String, String, Vec<u8>)>,
    page_rels: Vec<(usize, String, String, String)>,
    defaults: Vec<(String, String)>,
}

struct PageDef {
    id: u32,
    name: String,
    sheet: String,
    shapes: String,
    background: bool,
    back_page: Option<u32>,
}

impl Doc {
    fn new() -> Doc {
        Doc { pages: Vec::new(), theme: None, parts: Vec::new(), page_rels: Vec::new(), defaults: Vec::new() }
    }

    fn page(&mut self, name: &str, w: f64, h: f64, extra: &str, shapes: String) -> usize {
        let id = self.pages.len() as u32;
        let sheet = format!("<Cell N='PageWidth' V='{w}'/><Cell N='PageHeight' V='{h}'/><Cell N='PageScale' V='1'/><Cell N='DrawingScale' V='1'/>{extra}");
        self.pages.push(PageDef { id, name: String::from(name), sheet, shapes, background: false, back_page: None });
        self.pages.len() - 1
    }

    fn pkg(self) -> Pkg {
        let mut p = Pkg::new();
        for (e, t) in &self.defaults {
            p.default_type(e, t);
        }
        p.rel("", "rId1", "document", "visio/document.xml");
        p.part("visio/document.xml", CT_DRAWING, format!("{DECL}<VisioDocument {NS}><DocumentSettings DefaultTextStyle='0' DefaultLineStyle='0' DefaultFillStyle='0' DefaultGuideStyle='0'/>\
<Colors><ColorEntry IX='24' RGB='#2E75B6'/></Colors><FaceNames><FaceName ID='1' Name='Courier New' NameU='Courier New'/></FaceNames>{STYLES}</VisioDocument>"));
        p.rel("visio/document.xml", "rId1", "pages", "pages/pages.xml");
        if let Some(t) = &self.theme {
            p.rel("visio/document.xml", "rId2", "theme", "theme/theme1.xml");
            p.part("visio/theme/theme1.xml", CT_THEME, format!("{DECL}{t}"));
        }
        let mut pages = format!("{DECL}<Pages {NS}>");
        for (i, pg) in self.pages.iter().enumerate() {
            let bg = if pg.background { " Background='1'" } else { "" };
            let back = pg.back_page.map(|b| format!(" BackPage='{b}'")).unwrap_or_default();
            pages.push_str(&format!("<Page ID='{}' NameU='{}' Name='{}'{bg}{back}><PageSheet LineStyle='0' FillStyle='0' TextStyle='0'>{}</PageSheet><Rel r:id='rId{}'/></Page>", pg.id, pg.name, pg.name, pg.sheet, i + 1));
            p.rel("visio/pages/pages.xml", &format!("rId{}", i + 1), "page", &format!("page{}.xml", i + 1));
            p.part(&format!("visio/pages/page{}.xml", i + 1), CT_PAGE, format!("{DECL}<PageContents {NS}><Shapes>{}</Shapes></PageContents>", pg.shapes));
        }
        pages.push_str("</Pages>");
        p.part("visio/pages/pages.xml", CT_PAGES, pages);
        for (n, t, d) in self.parts {
            p.part(&n, &t, d);
        }
        for (i, id, kind, target) in self.page_rels {
            p.rel(&format!("visio/pages/page{}.xml", i + 1), &id, &kind, &target);
        }
        p
    }
}

// ---- shape helpers

fn c(n: &str, v: impl core::fmt::Display) -> String {
    format!("<Cell N='{n}' V='{v}'/>")
}

/// a 2D shape: pin, size (local pin in the middle), more cells, sections, text (raw)
fn shape(id: u32, pin: (f64, f64), size: (f64, f64), cells: &str, sections: &str, text: &str) -> String {
    let text = if text.is_empty() { String::new() } else { format!("<Text>{text}</Text>") };
    format!(
        "<Shape ID='{id}' NameU='Shape.{id}' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{cells}{sections}{text}</Shape>",
        c("PinX", pin.0),
        c("PinY", pin.1),
        c("Width", size.0),
        c("Height", size.1),
        c("LocPinX", size.0 * 0.5),
        c("LocPinY", size.1 * 0.5)
    )
}

/// a Geometry section of rows (T, cells)
fn geo(ix: u32, flags: &str, rows: &[(&str, String)]) -> String {
    let mut s = format!("<Section N='Geometry' IX='{ix}'>{flags}");
    for (i, (t, cells)) in rows.iter().enumerate() {
        s.push_str(&format!("<Row T='{t}' IX='{}'>{cells}</Row>", i + 1));
    }
    s.push_str("</Section>");
    s
}

fn xy(x: f64, y: f64) -> String {
    format!("{}{}", c("X", x), c("Y", y))
}

fn rect(w: f64, h: f64) -> String {
    geo(0, "", &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(w, 0.0)), ("LineTo", xy(w, h)), ("LineTo", xy(0.0, h)), ("LineTo", xy(0.0, 0.0))])
}

/// an L: asymmetric, so flips and turns show
fn ell(w: f64, h: f64) -> String {
    geo(0, "", &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(w, 0.0)), ("LineTo", xy(w, h * 0.3)), ("LineTo", xy(w * 0.3, h * 0.3)), ("LineTo", xy(w * 0.3, h)), ("LineTo", xy(0.0, h)), ("LineTo", xy(0.0, 0.0))])
}

// ---- the drawings

fn geometry() -> Pkg {
    let mut s = String::new();
    // MoveTo / LineTo
    s += &shape(1, (1.0, 5.0), (1.2, 1.0), &c("FillForegnd", "#ffd966"), &geo(0, "", &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(1.2, 0.0)), ("LineTo", xy(0.6, 1.0)), ("LineTo", xy(0.0, 0.0))]), "");
    // ArcTo: a box with rounded corners (bow r (1 - cos 45))
    let b = 0.2 * (1.0 - core::f64::consts::FRAC_1_SQRT_2);
    s += &shape(2, (3.0, 5.0), (1.5, 1.0), &c("FillForegnd", "#9dc3e6"), &geo(0, "", &[
        ("MoveTo", xy(0.2, 0.0)), ("LineTo", xy(1.3, 0.0)), ("ArcTo", format!("{}{}", xy(1.5, 0.2), c("A", b))), ("LineTo", xy(1.5, 0.8)),
        ("ArcTo", format!("{}{}", xy(1.3, 1.0), c("A", b))), ("LineTo", xy(0.2, 1.0)), ("ArcTo", format!("{}{}", xy(0.0, 0.8), c("A", b))),
        ("LineTo", xy(0.0, 0.2)), ("ArcTo", format!("{}{}", xy(0.2, 0.0), c("A", b)))]), "");
    // EllipticalArcTo: half an ellipse twice as wide as high, closed by a line
    s += &shape(3, (5.0, 5.0), (1.6, 0.8), &c("FillForegnd", "#c5e0b4"), &geo(0, "", &[
        ("MoveTo", xy(0.0, 0.0)), ("EllipticalArcTo", format!("{}{}{}{}{}", xy(1.6, 0.0), c("A", 0.8), c("B", 0.8), c("C", 0), c("D", 2))), ("LineTo", xy(0.0, 0.0))]), "");
    // Ellipse
    s += &shape(4, (7.0, 5.0), (1.0, 0.7), &c("FillForegnd", "#f4b183"), &geo(0, "", &[("Ellipse", format!("{}{}{}{}{}", xy(0.5, 0.35), c("A", 1.0), c("B", 0.35), c("C", 0.5), c("D", 0.7)))]), "");
    // NURBSTo: a cubic S, relative inner points, open (not filled)
    s += &shape(5, (1.0, 3.0), (1.4, 1.0), &format!("{}{}", c("LineWeight", 0.03), c("LineColor", "#c00000")), &geo(0, &c("NoFill", 1), &[
        ("MoveTo", xy(0.0, 0.0)), ("NURBSTo", format!("{}{}{}{}{}<Cell N='E' V='NURBS(1, 3, 0, 0, 0.1,1,0,1, 0.9,0,0,1)'/>", xy(1.4, 1.0), c("A", 0), c("B", 1), c("C", 0), c("D", 1)))]), "");
    // PolylineTo: a zigzag, closed
    s += &shape(6, (3.0, 3.0), (1.4, 1.0), &c("FillForegnd", "#ffe699"), &geo(0, "", &[
        ("MoveTo", xy(0.0, 0.0)), ("PolylineTo", format!("{}<Cell N='A' V='POLYLINE(0, 0, 0.25,1, 0.5,0.2, 0.75,1, 1,0)' U='POLYLINE'/>", xy(0.0, 0.0)))]), "");
    // SplineStart / SplineKnot: a quadratic spline through clamped knots, open
    s += &shape(7, (5.0, 3.0), (1.4, 1.0), &c("LineColor", "#7030a0"), &geo(0, &c("NoFill", 1), &[
        ("MoveTo", xy(0.0, 0.0)), ("SplineStart", format!("{}{}{}{}{}", xy(0.35, 1.0), c("A", 0), c("B", 0), c("C", 2), c("D", 2))),
        ("SplineKnot", format!("{}{}", xy(1.05, 0.0), c("A", 0))), ("SplineKnot", format!("{}{}", xy(1.4, 1.0), c("A", 1)))]), "");
    // the relative rows
    s += &shape(8, (7.0, 3.0), (1.2, 1.0), &c("FillForegnd", "#bdd7ee"), &geo(0, "", &[
        ("RelMoveTo", xy(0.0, 0.0)), ("RelLineTo", xy(0.6, 0.0)),
        ("RelCubBezTo", format!("{}{}{}{}{}", xy(1.0, 0.5), c("A", 0.9), c("B", 0.0), c("C", 1.1), c("D", 0.3))),
        ("RelQuadBezTo", format!("{}{}{}", xy(0.5, 1.0), c("A", 1.0), c("B", 1.0))),
        ("RelEllipticalArcTo", format!("{}{}{}{}{}", xy(0.0, 0.0), c("A", 0.1), c("B", 0.6), c("C", 0), c("D", 1))), ]), "");
    // two sections, even-odd: a frame; a third section hidden (NoShow), a fourth without line
    s += &shape(9, (1.0, 1.2), (1.2, 1.2), &c("FillForegnd", "#a9d18e"),
        &format!("{}{}{}", rect(1.2, 1.2), geo(1, "", &[("MoveTo", xy(0.3, 0.3)), ("LineTo", xy(0.9, 0.3)), ("LineTo", xy(0.9, 0.9)), ("LineTo", xy(0.3, 0.9)), ("LineTo", xy(0.3, 0.3))]),
            geo(2, &c("NoShow", 1), &[("MoveTo", xy(-1.0, -1.0)), ("LineTo", xy(3.0, 3.0))])), "");
    // an open path with a fill colour: Visio fills only closed paths
    s += &shape(10, (3.0, 1.2), (1.2, 1.0), &c("FillForegnd", "#ff0000"), &geo(0, "", &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(1.2, 0.0)), ("LineTo", xy(1.2, 1.0)), ("LineTo", xy(0.0, 1.0))]), "");
    // NoLine and NoFill sections side by side
    s += &shape(11, (5.0, 1.2), (1.2, 1.0), &c("FillForegnd", "#8faadc"),
        &format!("{}{}", geo(0, &c("NoLine", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(0.5, 0.0)), ("LineTo", xy(0.5, 1.0)), ("LineTo", xy(0.0, 1.0)), ("LineTo", xy(0.0, 0.0))]),
            geo(1, &c("NoFill", 1), &[("MoveTo", xy(0.7, 0.0)), ("LineTo", xy(1.2, 0.0)), ("LineTo", xy(1.2, 1.0)), ("LineTo", xy(0.7, 1.0)), ("LineTo", xy(0.7, 0.0))])), "");
    // an infinite line, dashed, across the bottom
    s += &shape(12, (7.0, 1.2), (1.0, 1.0), &format!("{}{}", c("LinePattern", 2), c("LineColor", "#808080")), &geo(0, &c("NoFill", 1), &[("InfiniteLine", format!("{}{}{}", xy(0.0, -0.8), c("A", 1.0), c("B", -0.8)))]), "");
    let mut d = Doc::new();
    d.page("Geometrie", 8.0, 6.0, "", s);
    d.pkg()
}

fn transform() -> Pkg {
    let mut s = String::new();
    let deg = |d: f64| d * core::f64::consts::PI / 180.0;
    let blue = c("FillForegnd", "#9dc3e6");
    // unturned reference, turned 30 degrees, flipped in x, flipped in y, a local pin at the corner turned 45
    s += &shape(1, (1.0, 4.0), (1.0, 1.0), &blue, &ell(1.0, 1.0), "L");
    s += &shape(2, (2.6, 4.0), (1.0, 1.0), &format!("{blue}{}", c("Angle", deg(30.0))), &ell(1.0, 1.0), "30");
    s += &shape(3, (4.2, 4.0), (1.0, 1.0), &format!("{blue}{}", c("FlipX", 1)), &ell(1.0, 1.0), "FlipX");
    s += &shape(4, (5.8, 4.0), (1.0, 1.0), &format!("{blue}{}", c("FlipY", 1)), &ell(1.0, 1.0), "FlipY");
    s += &format!("<Shape ID='5' NameU='Corner' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{}{}{}</Shape>",
        c("PinX", 7.0), c("PinY", 3.5), c("Width", 1.0), c("Height", 1.0), c("LocPinX", 0), c("LocPinY", 0), c("Angle", deg(45.0)), c("FillForegnd", "#f4b183"), ell(1.0, 1.0));
    // a group turned 20 degrees: a child box and an L turned -45 and flipped, the group's own text
    let child1 = shape(7, (0.6, 0.6), (0.8, 0.6), &c("FillForegnd", "#c5e0b4"), &rect(0.8, 0.6), "a");
    let child2 = shape(8, (1.8, 0.9), (0.8, 0.8), &format!("{}{}{}", c("FillForegnd", "#ffd966"), c("Angle", deg(-45.0)), c("FlipX", 1)), &ell(0.8, 0.8), "b");
    s += &format!("<Shape ID='6' NameU='Group' Type='Group' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{}{}<Shapes>{child1}{child2}</Shapes><Text>Gruppe</Text></Shape>",
        c("PinX", 2.0), c("PinY", 1.5), c("Width", 2.4), c("Height", 1.5), c("LocPinX", 1.2), c("LocPinY", 0.75), c("Angle", deg(20.0)), geo(0, &c("NoFill", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(2.4, 0.0)), ("LineTo", xy(2.4, 1.5)), ("LineTo", xy(0.0, 1.5)), ("LineTo", xy(0.0, 0.0))]));
    // a 1D shape: begin (4, 0.8), end (7.2, 3.2) (3, 4, 5: length 4), its pin, width and angle as
    // Visio caches them
    let (bx, by, ex, ey) = (4.0f64, 0.8f64, 7.2f64, 3.2f64);
    let len = 4.0;
    let ang = 0.643_501_108_793_284_4;
    s += &format!("<Shape ID='9' NameU='Line' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}</Shape>",
        c("PinX", (bx + ex) * 0.5), c("PinY", (by + ey) * 0.5), c("Width", len), c("Height", 0), c("LocPinX", len * 0.5), c("LocPinY", 0), c("Angle", ang),
        c("BeginX", bx), c("BeginY", by), c("EndX", ex), c("EndY", ey), c("EndArrow", 13), c("LineWeight", 0.02),
        geo(0, &c("NoFill", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(len, 0.0))]), "<Text>1D</Text>");
    let mut d = Doc::new();
    d.page("Transform", 8.0, 5.0, "", s);
    d.pkg()
}

fn lines() -> Pkg {
    let mut s = String::new();
    let line = |id: u32, y: f64, x0: f64, x1: f64, cells: &str| {
        let w = x1 - x0;
        format!("<Shape ID='{id}' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{cells}{}</Shape>",
            c("PinX", (x0 + x1) * 0.5), c("PinY", y), c("Width", w), c("Height", 0), c("LocPinX", w * 0.5), c("LocPinY", 0),
            geo(0, &c("NoFill", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(w, 0.0))]))
    };
    // patterns 1 .. 12 at 1 pt
    for k in 1..=12u32 {
        s += &line(k, 5.6 - k as f64 * 0.4, 0.4, 3.4, &format!("{}{}", c("LinePattern", k), c("LineWeight", 1.0 / 72.0)));
    }
    // weights and caps (round, square = butt, extended)
    for (k, cap) in [0u32, 1, 2].iter().enumerate() {
        s += &line(20 + k as u32, 5.2 - k as f64 * 0.5, 4.0, 5.6, &format!("{}{}{}", c("LineWeight", 0.12), c("LineCap", cap), c("LineColor", "#2E75B6")));
    }
    // a thin guide over the caps' ends
    s += &line(23, 4.7, 4.0, 4.0001, &c("LineColor", "#ff0000"));
    // line ends: kinds at size 2, then sizes 0 .. 6 of kind 13
    for (k, kind) in [1u32, 2, 4, 5, 10, 12, 14, 16, 22, 24, 29, 30, 31, 32, 33].iter().enumerate() {
        let y = 5.4 - k as f64 * 0.3;
        s += &line(30 + k as u32, y, 6.2, 7.6, &format!("{}{}", c("BeginArrow", kind), c("EndArrow", kind)));
    }
    for size in 0..=6u32 {
        s += &line(50 + size, 3.2 - size as f64 * 0.3, 4.0, 5.6, &format!("{}{}", c("EndArrow", 13), c("EndArrowSize", size)));
    }
    // rounded corners and a translucent thick line over another
    s += &shape(60, (1.8, 0.6), (2.4, 0.8), &format!("{}{}{}", c("Rounding", 0.15), c("FillPattern", 0), c("LineWeight", 0.03)),
        &geo(0, &c("NoFill", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(0.8, 0.8)), ("LineTo", xy(1.6, 0.0)), ("LineTo", xy(2.4, 0.8))]), "");
    s += &line(61, 0.6, 4.0, 7.6, &format!("{}{}", c("LineWeight", 0.1), c("LineColor", "#000000")));
    s += &shape(62, (5.8, 0.6), (0.8, 0.8), &format!("{}{}{}{}", c("LineWeight", 0.12), c("LineColor", "#c00000"), c("LineColorTrans", 0.5), c("FillPattern", 0)), &rect(0.8, 0.8), "");
    let mut d = Doc::new();
    d.page("Linien", 8.0, 6.0, "", s);
    d.pkg()
}

fn fills() -> Pkg {
    let mut s = String::new();
    let box_ = |id: u32, x: f64, y: f64, cells: String| shape(id, (x, y), (1.0, 0.8), &cells, &rect(1.0, 0.8), "");
    // translucent overlapping fills
    s += &box_(1, 0.9, 5.0, format!("{}{}", c("FillForegnd", "#ff0000"), c("FillForegndTrans", 0.5)));
    s += &box_(2, 1.4, 4.6, format!("{}{}", c("FillForegnd", "#0000ff"), c("FillForegndTrans", 0.5)));
    // hatches 2 .. 7 over a light background
    for k in 2..=7u32 {
        s += &box_(k + 1, 2.3 + (k - 2) as f64 * 1.05, 5.0, format!("{}{}{}", c("FillPattern", k), c("FillForegnd", "#1f4e79"), c("FillBkgnd", "#deebf7")));
    }
    // the old gradients 25, 29, 35
    for (k, p) in [25u32, 29, 35].iter().enumerate() {
        s += &box_(10 + k as u32, 0.9 + k as f64 * 1.3, 3.5, format!("{}{}{}", c("FillPattern", p), c("FillForegnd", "#ffc000"), c("FillBkgnd", "#ffffff")));
    }
    // FillGradient: linear at 0 degrees and radial
    let stops = "<Section N='FillGradient'><Row IX='0'><Cell N='GradientStopColor' V='#c00000'/><Cell N='GradientStopColorTrans' V='0'/><Cell N='GradientStopPosition' V='0'/></Row>\
<Row IX='1'><Cell N='GradientStopColor' V='#ffff00'/><Cell N='GradientStopColorTrans' V='0'/><Cell N='GradientStopPosition' V='0.5'/></Row>\
<Row IX='2'><Cell N='GradientStopColor' V='#00b050'/><Cell N='GradientStopColorTrans' V='0'/><Cell N='GradientStopPosition' V='1'/></Row></Section>";
    s += &shape(13, (4.9, 3.5), (1.2, 0.8), &format!("{}{}{}", c("FillGradientEnabled", 1), c("FillGradientDir", 0), c("FillGradientAngle", 0)), &format!("{}{stops}", rect(1.2, 0.8)), "");
    s += &shape(14, (6.4, 3.5), (1.2, 0.8), &format!("{}{}", c("FillGradientEnabled", 1), c("FillGradientDir", 1)), &format!("{}{stops}", rect(1.2, 0.8)), "");
    // a shadow
    s += &shape(15, (1.0, 1.9), (1.2, 0.8), &format!("{}{}{}{}{}", c("FillForegnd", "#a9d18e"), c("ShdwPattern", 1), c("ShdwForegnd", "#7f7f7f"), c("ShapeShdwOffsetX", 0.08), c("ShapeShdwOffsetY", -0.08)), &rect(1.2, 0.8), "Schatten");
    // layers: shape 16 on the hidden layer, 17 on the red layer, 18 on both visible and hidden
    s += &shape(16, (2.8, 1.9), (1.0, 0.8), &format!("{}{}", c("FillForegnd", "#000000"), "<Cell N='LayerMember' V='1'/>"), &rect(1.0, 0.8), "verborgen");
    s += &shape(17, (4.2, 1.9), (1.0, 0.8), &format!("{}{}", c("FillForegnd", "#ffffff"), "<Cell N='LayerMember' V='2'/>"), &rect(1.0, 0.8), "rot");
    s += &shape(18, (5.6, 1.9), (1.0, 0.8), &format!("{}{}", c("FillForegnd", "#c5e0b4"), "<Cell N='LayerMember' V='0;1'/>"), &rect(1.0, 0.8), "zwei");
    // colour index 24 of the document's table, and transparency of the background in a hatch
    s += &box_(19, 7.0, 1.9, format!("{}", c("FillForegnd", 24)));
    let layers = "<Section N='Layer'><Row IX='0'><Cell N='Name' V='Sichtbar'/><Cell N='Visible' V='1'/><Cell N='Color' V='255'/></Row>\
<Row IX='1'><Cell N='Name' V='Versteckt'/><Cell N='Visible' V='0'/><Cell N='Color' V='255'/></Row>\
<Row IX='2'><Cell N='Name' V='Rot'/><Cell N='Visible' V='1'/><Cell N='Color' V='#ff0000'/><Cell N='ColorTrans' V='0'/></Row></Section>";
    let mut d = Doc::new();
    d.page("Fuellungen", 8.0, 6.0, layers, s);
    d.pkg()
}

fn text() -> Pkg {
    let mut s = String::new();
    let frame = |id: u32, x: f64, y: f64, w: f64, h: f64, cells: &str, t: &str| shape(id, (x, y), (w, h), cells, &rect(w, h), t);
    // vertical alignment top / middle / bottom, horizontal left / centre / right
    s += &frame(1, 1.0, 5.2, 1.6, 1.0, &format!("{}<Section N='Paragraph'><Row IX='0'><Cell N='HorzAlign' V='0'/></Row></Section>", c("VerticalAlign", 0)), "<cp IX='0'/><pp IX='0'/>oben links\n");
    s += &frame(2, 2.8, 5.2, 1.6, 1.0, &c("VerticalAlign", 1), "Mitte\n");
    s += &frame(3, 4.6, 5.2, 1.6, 1.0, &format!("{}<Section N='Paragraph'><Row IX='0'><Cell N='HorzAlign' V='2'/></Row></Section>", c("VerticalAlign", 2)), "unten rechts\n");
    // runs: bold, italic, colour, size, underline, strike; two paragraphs, a line break inside one
    let chars = "<Section N='Character'><Row IX='0'/>\
<Row IX='1'><Cell N='Style' V='1'/></Row><Row IX='2'><Cell N='Style' V='2'/><Cell N='Color' V='#c00000'/></Row>\
<Row IX='3'><Cell N='Size' V='0.25' U='PT'/><Cell N='Style' V='4'/></Row><Row IX='4'><Cell N='Strikethru' V='1'/><Cell N='Font' V='Times New Roman'/></Row>\
<Row IX='5'><Cell N='Font' V='1'/></Row></Section>\
<Section N='Paragraph'><Row IX='0'/><Row IX='1'><Cell N='HorzAlign' V='0'/><Cell N='SpBefore' V='0.05'/></Row></Section>";
    s += &frame(4, 6.6, 5.0, 2.4, 1.4, chars,
        "<cp IX='0'/><pp IX='0'/>normal <cp IX='1'/>fett <cp IX='2'/>kursiv rot\n<pp IX='1'/><cp IX='3'/>gross<cp IX='0'/> und\u{2028}<cp IX='4'/>Serif durch <cp IX='5'/>Courier\n");
    // wrapping in a narrow block
    s += &frame(5, 1.2, 3.2, 1.4, 1.4, "", "Ein langer Text, der in dem schmalen Block mehrfach umbricht.\n");
    // a turned block (TxtAngle) and a turned shape
    s += &format!("<Shape ID='6' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{}{}<Text>TxtAngle 90</Text></Shape>",
        c("PinX", 3.0), c("PinY", 3.2), c("Width", 1.2), c("Height", 1.2), c("LocPinX", 0.6), c("LocPinY", 0.6), c("TxtAngle", core::f64::consts::FRAC_PI_2), rect(1.2, 1.2));
    s += &frame(7, 4.6, 3.2, 1.4, 0.8, &c("Angle", 0.5), "gedreht\n");
    // flipped shapes: the text stays readable (FlipX) or turns over (FlipY)
    s += &frame(8, 6.4, 3.4, 1.4, 0.6, &c("FlipX", 1), "FlipX\n");
    s += &frame(9, 6.4, 2.6, 1.4, 0.6, &c("FlipY", 1), "FlipY\n");
    // vertical text, a field with its cached text, a text background on a line
    s += &frame(10, 0.8, 1.2, 0.8, 1.6, &c("TextDirection", 1), "senkrecht\n");
    s += &frame(11, 2.6, 1.2, 1.8, 0.8, "<Section N='Field'><Row IX='0'><Cell N='Value' V='17.11.2026' U='DATE' F='DOCCREATION()'/></Row></Section>", "Datum: <fld IX='0'>17.11.2026</fld>\n");
    s += &format!("<Shape ID='12' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{}{}</Shape>",
        c("PinX", 5.4), c("PinY", 1.2), c("Width", 2.4), c("Height", 0), c("LocPinX", 1.2), c("LocPinY", 0), c("TextBkgnd", "#ffffff"),
        format!("{}<Text>Beschriftung</Text>", geo(0, &c("NoFill", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(2.4, 0.0))])));
    // a big tab and a bullet
    s += &frame(13, 7.4, 1.2, 1.0, 1.0, "<Section N='Paragraph'><Row IX='0'><Cell N='Bullet' V='1'/><Cell N='HorzAlign' V='0'/><Cell N='IndLeft' V='0.15'/><Cell N='IndFirst' V='-0.15'/></Row></Section>", "eins\nzwei\n");
    let mut d = Doc::new();
    d.page("Text", 8.0, 6.0, "", s);
    d.pkg()
}

/// a small picture: a colour ramp with a dark frame, as PNG or JPEG
pub fn picture_bytes(jpeg: bool) -> Vec<u8> {
    let (w, h) = (48i32, 32i32);
    let mut sf = gfx::Surface::new(w, h, gfx::FMT_XRGB);
    for y in 0..h {
        for x in 0..w {
            let edge = x < 2 || y < 2 || x >= w - 2 || y >= h - 2;
            let (r, g, b) = if edge { (20, 20, 60) } else { ((x * 255 / w) as u32, (y * 255 / h) as u32, 160) };
            let o = (y * sf.stride + x * 4) as usize;
            sf.px[o..o + 4].copy_from_slice(&[b as u8, g as u8, r as u8, 255]);
        }
    }
    if jpeg {
        img_jpeg::enc::encode(&sf, 95).unwrap_or_default()
    } else {
        img_png::encode(&sf.px, w as usize, h as usize, sf.stride as usize, false)
    }
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= ch.len() {
                s.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn pictures() -> Pkg {
    let png = picture_bytes(false);
    let jpg = picture_bytes(true);
    // an EMF in signature only: a placeholder
    let mut emf = alloc::vec![0u8; 88];
    emf[40..44].copy_from_slice(b" EMF");
    let foreign = |id: u32, pin: (f64, f64), size: (f64, f64), cells: &str, data: &str| {
        format!("<Shape ID='{id}' NameU='Picture.{id}' Type='Foreign' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{cells}{}{data}</Shape>",
            c("PinX", pin.0), c("PinY", pin.1), c("Width", size.0), c("Height", size.1), c("LocPinX", size.0 * 0.5), c("LocPinY", size.1 * 0.5),
            geo(0, &format!("{}{}", c("NoFill", 1), c("NoLine", 1)), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(size.0, 0.0)), ("LineTo", xy(size.0, size.1)), ("LineTo", xy(0.0, size.1)), ("LineTo", xy(0.0, 0.0))]))
    };
    let mut s = String::new();
    s += &foreign(1, (1.2, 3.0), (1.8, 1.2), "", &format!("<ForeignData ForeignType='Bitmap' CompressionType='PNG'>{}</ForeignData>", base64(&png)));
    s += &foreign(2, (3.4, 3.0), (1.8, 1.2), "", "<ForeignData ForeignType='Bitmap' CompressionType='PNG'><Rel r:id='rId1'/></ForeignData>");
    s += &foreign(3, (5.6, 3.0), (1.8, 1.2), "", "<ForeignData ForeignType='Bitmap' CompressionType='JPEG'><Rel r:id='rId2'/></ForeignData>");
    s += &foreign(4, (1.2, 1.2), (1.8, 1.2), "", "<ForeignData ForeignType='EnhMetaFile'><Rel r:id='rId3'/></ForeignData>");
    // cut: the picture twice the box, moved left and down
    s += &foreign(5, (3.4, 1.2), (1.2, 1.0), &format!("{}{}{}{}", c("ImgOffsetX", -0.6), c("ImgOffsetY", -0.5), c("ImgWidth", 2.4), c("ImgHeight", 2.0)), "<ForeignData ForeignType='Bitmap' CompressionType='PNG'><Rel r:id='rId1'/></ForeignData>");
    // turned and with a frame
    s += &format!("<Shape ID='6' Type='Foreign' LineStyle='0' FillStyle='0' TextStyle='0'>{}{}{}{}{}{}{}{}{}{}</Shape>",
        c("PinX", 5.6), c("PinY", 1.2), c("Width", 1.5), c("Height", 1.0), c("LocPinX", 0.75), c("LocPinY", 0.5), c("Angle", 0.3), c("LineWeight", 0.02),
        geo(0, &c("NoFill", 1), &[("MoveTo", xy(0.0, 0.0)), ("LineTo", xy(1.5, 0.0)), ("LineTo", xy(1.5, 1.0)), ("LineTo", xy(0.0, 1.0)), ("LineTo", xy(0.0, 0.0))]),
        "<ForeignData ForeignType='Bitmap' CompressionType='PNG'><Rel r:id='rId1'/></ForeignData>");
    let mut d = Doc::new();
    d.defaults.push((String::from("png"), String::from("image/png")));
    d.defaults.push((String::from("jpeg"), String::from("image/jpeg")));
    d.defaults.push((String::from("emf"), String::from("image/x-emf")));
    d.page("Bilder", 7.0, 4.0, "", s);
    d.parts.push((String::from("visio/media/image1.png"), String::new(), png));
    d.parts.push((String::from("visio/media/image2.jpeg"), String::new(), jpg));
    d.parts.push((String::from("visio/media/image3.emf"), String::new(), emf));
    d.page_rels.push((0, String::from("rId1"), String::from("image"), String::from("../media/image1.png")));
    d.page_rels.push((0, String::from("rId2"), String::from("image"), String::from("../media/image2.jpeg")));
    d.page_rels.push((0, String::from("rId3"), String::from("image"), String::from("../media/image3.emf")));
    d.pkg()
}

/// a theme of our own: accent 1 dark red, variation colour 1 teal, background light yellow
pub const THEME: &str = "<a:theme xmlns:a='http://schemas.openxmlformats.org/drawingml/2006/main' name='Probe'><a:themeElements>\
<a:clrScheme name='Probe'><a:dk1><a:srgbClr val='202020'/></a:dk1><a:lt1><a:srgbClr val='FFFFFF'/></a:lt1><a:dk2><a:srgbClr val='333366'/></a:dk2>\
<a:lt2><a:srgbClr val='EEEEEE'/></a:lt2><a:accent1><a:srgbClr val='C00000'/></a:accent1><a:accent2><a:srgbClr val='0070C0'/></a:accent2>\
<a:accent3><a:srgbClr val='00B050'/></a:accent3><a:accent4><a:srgbClr val='7030A0'/></a:accent4><a:accent5><a:srgbClr val='FFC000'/></a:accent5>\
<a:accent6><a:srgbClr val='808080'/></a:accent6><a:hlink><a:srgbClr val='0563C1'/></a:hlink><a:folHlink><a:srgbClr val='954F72'/></a:folHlink>\
<a:extLst><a:ext uri='{1}'><vt:schemeID xmlns:vt='http://schemas.microsoft.com/office/visio/2012/theme' schemeEnum='77'/></a:ext>\
<a:ext uri='{2}'><vt:bkgnd xmlns:vt='http://schemas.microsoft.com/office/visio/2012/theme'><a:srgbClr val='FFFFE0'/></vt:bkgnd></a:ext>\
<a:ext uri='{3}'><vt:variationClrSchemeLst xmlns:vt='http://schemas.microsoft.com/office/visio/2012/theme'><vt:variationClrScheme>\
<vt:varColor1><a:srgbClr val='008080'/></vt:varColor1></vt:variationClrScheme></vt:variationClrSchemeLst></a:ext></a:extLst></a:clrScheme>\
<a:fontScheme name='Probe'><a:majorFont><a:latin typeface='Georgia'/></a:majorFont><a:minorFont><a:latin typeface='Consolas'/></a:minorFont></a:fontScheme>\
<a:fmtScheme name='Probe'><a:lnStyleLst><a:ln w='38100'/><a:ln w='19050'/></a:lnStyleLst></a:fmtScheme></a:themeElements>\
<a:extLst><a:ext uri='{4}'><vt:themeScheme xmlns:vt='http://schemas.microsoft.com/office/visio/2012/theme'><vt:schemeID schemeEnum='77'/></vt:themeScheme></a:ext></a:extLst></a:theme>";

fn pages() -> Pkg {
    let mut d = Doc::new();
    d.theme = Some(String::from(THEME));
    // page 1: themed values, on a background page
    let mut s = String::new();
    s += &shape(1, (2.0, 2.0), (2.0, 1.2), &format!("{}{}{}{}{}",
        "<Cell N='FillForegnd' V='Themed' F='THEMEVAL()'/>", "<Cell N='LineColor' V='Themed' F='THEMEVAL()'/>", "<Cell N='LineWeight' V='Themed' F='THEMEVAL()'/>",
        c("QuickStyleFillColor", 2), c("QuickStyleLineColor", 100)),
        &format!("{}<Section N='Character'><Row IX='0'><Cell N='Font' V='Themed' F='THEMEVAL()'/><Cell N='Color' V='Themed' F='THEMEVAL()'/></Row></Section>", rect(2.0, 1.2)),
        "Design\n");
    s += &shape(2, (4.6, 2.0), (1.6, 1.2), &c("FillForegnd", "#9dc3e6"), &rect(1.6, 1.2), "Vordergrund\n");
    let i = d.page("Seite-1", 6.0, 4.0, &format!("{}", c("ThemeIndex", 77)), s);
    d.pages[i].back_page = Some(2);
    // page 2: drawn at 1:10 (shapes in drawing units ten times larger, lines and text as on paper)
    let mut s2 = String::new();
    s2 += &shape(1, (20.0, 20.0), (20.0, 10.0), &c("FillForegnd", "#c5e0b4"), &rect(20.0, 10.0), "1:10\n");
    s2 += &shape(2, (45.0, 20.0), (10.0, 10.0), &c("FillForegnd", "#ffd966"), &geo(0, "", &[("Ellipse", format!("{}{}{}{}{}", xy(5.0, 5.0), c("A", 10.0), c("B", 5.0), c("C", 5.0), c("D", 10.0)))]), "");
    d.page("Massstab", 60.0, 40.0, "<Cell N='DrawingScale' V='10'/><Cell N='PageScale' V='1'/>", s2);
    // fix the second page's scale: DrawingScale 10 drawing inches per PageScale 1 page inch
    let p2 = &mut d.pages[1];
    p2.sheet = format!("<Cell N='PageWidth' V='60'/><Cell N='PageHeight' V='40'/><Cell N='PageScale' V='1'/><Cell N='DrawingScale' V='10'/>");
    // the background page: a frame and a title line
    let mut s3 = String::new();
    s3 += &shape(1, (3.0, 2.0), (5.8, 3.8), &format!("{}{}", c("FillPattern", 0), c("LineColor", "#7f7f7f")), &rect(5.8, 3.8), "");
    s3 += &shape(2, (3.0, 3.7), (5.8, 0.3), &c("FillForegnd", "#d9d9d9"), &rect(5.8, 0.3), "Hintergrund\n");
    let b = d.page("Hintergrund-1", 6.0, 4.0, "", s3);
    d.pages[b].background = true;
    d.pages[b].id = 2;
    d.pkg()
}
