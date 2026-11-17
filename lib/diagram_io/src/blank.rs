// blank -- new documents from scratch: drawing, template, stencil (vsdx::new_document)
//
// Everything here is JFLib's own: the style sheets and the document sheet hold values chosen
// for the NetLab look, the theme is the own DrawingML theme "NetLab" (theme.rs); nothing comes
// from Visio's files.  What a file needs so that Visio opens it follows the public [MS-VSDX]:
// the document part with its root style sheet ID 0 "No Style" (every other style inherits from
// it), Colors and FaceNames with at least one entry each, a Pages part with one page whose
// PageSheet is there (a drawing's and template's page has its own part, a stencil's none), the
// theme part named by the document part, the document properties (optional, written with the
// application's name only: no user data).
//
// The styles (IDs as Visio's own documents number them, universal names as Visio knows them,
// so that formulas and Visio's connector detection find them):
//   0 No Style   the root: every line, fill, text block, character and paragraph cell with a
//                plain value (anthracite 0.75 pt line, white fill, Inter 10 pt, centred text)
//   1 Normal     based on No Style: the NetLab shape look (Signalblau 1 pt line, light blue fill)
//   2 Connector  based on Normal: grey 1 pt line, no fill, white text background
//   3 Guide      based on No Style: thin light blue dashed line, no fill
//   4 Text Only  based on Normal: no line, no fill
// DocumentSettings name Normal as the default text, line and fill style and Guide as the
// default guide style.  The page is A4 (metric, millimetres) or Letter (US, inches), landscape.

use alloc::vec::Vec;

use diagram_core::{
    Cell, ColorEntry, DocKind, DocSettings, Document, FaceName, Formula, Page, PageHeader, Part, Row, RowKey, SecKey, Section,
    Sheet, Style, StyleHeader, StyleRefs, Theme, Unit, Value,
};

use crate::theme;
use crate::xml::{NS_REL, NS_VISIO};

/// the measuring system of a new document (FEATURES LG-07, MS-07)
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Measure {
    /// millimetres, A4
    #[default]
    Metric,
    /// inches, Letter
    Us,
}

// the legacy colour indexes 0-23 as [MS-VSDX] fixes them (Color Table); written so that the
// Colors element of a new document is never empty
const COLORS: [&str; 24] = [
    "#000000", "#FFFFFF", "#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FF00FF", "#00FFFF", "#800000", "#008000", "#000080", "#808000",
    "#800080", "#008080", "#C0C0C0", "#E6E6E6", "#CDCDCD", "#B3B3B3", "#9A9A9A", "#808080", "#666666", "#4D4D4D", "#333333", "#1A1A1A",
];

// the NetLab colours the styles use (styles/netlab: palette.rs, lib.rs)
const ANTHRACITE: &str = "#1D2228"; // text primary
const SLATE: &str = "#4D5560"; // text secondary
const SIGNALBLAU: &str = "#1F5FA8"; // the accent
const SIGNALBLAU_LIGHT: &str = "#6FA3E0"; // the accent of the dark theme
const INFO_BG: &str = "#E6EFF8"; // the info background
const WHITE: &str = "#FFFFFF";

/// the typeface of the NetLab style
pub(crate) const FONT: &str = "Inter";

const PT: f64 = 1.0 / 72.0;
const MM: f64 = 1.0 / 25.4;

// a cell value as the builder states it
enum V {
    // a number in internal units with the unit it is shown in
    N(f64, Option<Unit>),
    // text (a colour, a font name)
    T(&'static str),
    // text with a formula (F)
    F(&'static str, &'static str),
}

fn cell(doc: &mut Document, name: &str, v: V) -> Cell {
    let n = doc.intern(name);
    match v {
        V::N(x, u) => Cell::constant(n, Value::Num(x), u),
        V::T(t) => {
            let t = doc.intern(t);
            Cell::constant(n, Value::Text(t), None)
        }
        V::F(t, f) => {
            let t = doc.intern(t);
            let f = doc.intern(f);
            Cell { formula: Formula::Expr(f), ..Cell::constant(n, Value::Text(t), None) }
        }
    }
}

fn cells(doc: &mut Document, list: Vec<(&str, V)>) -> Vec<Cell> {
    list.into_iter().map(|(n, v)| cell(doc, n, v)).collect()
}

// a section of one indexed row (Character, Paragraph)
fn one_row(doc: &mut Document, section: &str, list: Vec<(&str, V)>) -> Section {
    let name = doc.intern(section);
    let mut sec = Section::new(SecKey::one(name));
    let mut row = Row::new(RowKey::Index(0));
    row.cells = cells(doc, list);
    sec.rows.push(row);
    sec
}

fn num(x: f64) -> V {
    V::N(x, None)
}

fn pt(points: f64) -> V {
    V::N(points * PT, Some(Unit::Points))
}

fn style(doc: &mut Document, id: u32, name: &str, based_on: Option<u32>, sheet: Sheet) -> Style {
    let n = doc.intern(name);
    let header = StyleHeader {
        id,
        name: Some(n),
        name_u: Some(n),
        based_on: based_on.map_or(StyleRefs::default(), StyleRefs::all),
        ..StyleHeader::default()
    };
    Style { header, sheet, extras: Default::default() }
}

// 0 No Style: the end of every inheritance chain, so every style cell has a value
fn no_style(doc: &mut Document) -> Style {
    let mut sh = Sheet::new();
    sh.cells = cells(
        doc,
        alloc::vec![
            ("EnableLineProps", num(1.0)),
            ("EnableFillProps", num(1.0)),
            ("EnableTextProps", num(1.0)),
            ("HideForApply", num(0.0)),
            // line
            ("LineWeight", pt(0.75)),
            ("LineColor", V::T(ANTHRACITE)),
            ("LinePattern", num(1.0)),
            ("Rounding", V::N(0.0, Some(Unit::DrawingLength))),
            ("EndArrowSize", num(2.0)),
            ("BeginArrow", num(0.0)),
            ("EndArrow", num(0.0)),
            ("LineCap", num(0.0)),
            ("BeginArrowSize", num(2.0)),
            ("LineColorTrans", V::N(0.0, Some(Unit::Percent))),
            ("CompoundType", num(0.0)),
            // fill
            ("FillForegnd", V::T(WHITE)),
            ("FillBkgnd", V::T(WHITE)),
            ("FillPattern", num(1.0)),
            ("ShdwForegnd", V::T(ANTHRACITE)),
            ("ShdwPattern", num(0.0)),
            ("FillForegndTrans", V::N(0.0, Some(Unit::Percent))),
            ("FillBkgndTrans", V::N(0.0, Some(Unit::Percent))),
            ("ShdwForegndTrans", V::N(0.0, Some(Unit::Percent))),
            ("ShapeShdwType", num(0.0)),
            ("ShapeShdwOffsetX", V::N(2.0 * MM, Some(Unit::Millimeters))),
            ("ShapeShdwOffsetY", V::N(-2.0 * MM, Some(Unit::Millimeters))),
            ("ShapeShdwObliqueAngle", V::N(0.0, Some(Unit::Degrees))),
            ("ShapeShdwScaleFactor", V::N(1.0, Some(Unit::Percent))),
            // text block
            ("LeftMargin", pt(4.0)),
            ("RightMargin", pt(4.0)),
            ("TopMargin", pt(4.0)),
            ("BottomMargin", pt(4.0)),
            ("VerticalAlign", num(1.0)),
            ("TextBkgnd", num(0.0)),
            ("TextBkgndTrans", V::N(0.0, Some(Unit::Percent))),
            ("TextDirection", num(0.0)),
        ],
    );
    let character = one_row(
        doc,
        "Character",
        alloc::vec![
            ("Font", V::T(FONT)),
            ("Color", V::T(ANTHRACITE)),
            ("Style", num(0.0)),
            ("Case", num(0.0)),
            ("Pos", num(0.0)),
            ("FontScale", V::N(1.0, Some(Unit::Percent))),
            ("Size", pt(10.0)),
            ("Letterspace", pt(0.0)),
            ("ColorTrans", V::N(0.0, Some(Unit::Percent))),
            ("Strikethru", num(0.0)),
            ("DblUnderline", num(0.0)),
            ("Overline", num(0.0)),
            ("DoubleStrikethrough", num(0.0)),
        ],
    );
    let paragraph = one_row(
        doc,
        "Paragraph",
        alloc::vec![
            ("IndFirst", pt(0.0)),
            ("IndLeft", pt(0.0)),
            ("IndRight", pt(0.0)),
            ("SpLine", V::N(-1.2, Some(Unit::Percent))),
            ("SpBefore", pt(0.0)),
            ("SpAfter", pt(0.0)),
            ("HorzAlign", num(1.0)),
            ("Bullet", num(0.0)),
            ("BulletStr", V::T("")),
            ("BulletFont", num(0.0)),
            ("BulletFontSize", num(-1.0)),
            ("TextPosAfterBullet", num(0.0)),
            ("Flags", num(0.0)),
        ],
    );
    sh.sections.push(character);
    sh.sections.push(paragraph);
    style(doc, 0, "No Style", None, sh)
}

fn styles(doc: &mut Document) -> Vec<Style> {
    let root = no_style(doc);
    let mut normal = Sheet::new();
    normal.cells = cells(doc, alloc::vec![("LineWeight", pt(1.0)), ("LineColor", V::T(SIGNALBLAU)), ("FillForegnd", V::T(INFO_BG)), ("FillBkgnd", V::T(INFO_BG))]);
    let mut connector = Sheet::new();
    connector.cells = cells(
        doc,
        alloc::vec![("LineWeight", pt(1.0)), ("LineColor", V::T(SLATE)), ("FillPattern", num(0.0)), ("TextBkgnd", V::F(WHITE, "RGB(255,255,255)"))],
    );
    let mut guide = Sheet::new();
    guide.cells = cells(
        doc,
        alloc::vec![("LineWeight", pt(0.0)), ("LineColor", V::T(SIGNALBLAU_LIGHT)), ("LinePattern", num(2.0)), ("FillPattern", num(0.0))],
    );
    let mut text_only = Sheet::new();
    text_only.cells = cells(doc, alloc::vec![("LinePattern", num(0.0)), ("FillPattern", num(0.0))]);
    alloc::vec![
        root,
        style(doc, 1, "Normal", Some(0), normal),
        style(doc, 2, "Connector", Some(1), connector),
        style(doc, 3, "Guide", Some(0), guide),
        style(doc, 4, "Text Only", Some(1), text_only),
    ]
}

// the page sheet of a new page: A4 or Letter landscape, scale 1:1, the size kept as set
fn page_sheet(doc: &mut Document, measure: Measure) -> Sheet {
    let (w, h, unit, paper) = match measure {
        Measure::Metric => (297.0 * MM, 210.0 * MM, Unit::Millimeters, 9.0),
        Measure::Us => (11.0, 8.5, Unit::Inches, 1.0),
    };
    let one = if measure == Measure::Metric { MM } else { 1.0 };
    let mut sh = Sheet::new();
    sh.cells = cells(
        doc,
        alloc::vec![
            ("PageWidth", V::N(w, Some(unit))),
            ("PageHeight", V::N(h, Some(unit))),
            ("PageScale", V::N(one, Some(unit))),
            ("DrawingScale", V::N(one, Some(unit))),
            ("DrawingSizeType", num(3.0)),
            ("DrawingScaleType", num(0.0)),
            ("InhibitSnap", num(0.0)),
            ("UIVisibility", num(0.0)),
            ("DrawingResizeType", num(1.0)),
            ("PageShapeSplit", num(1.0)),
            ("PrintPageOrientation", num(2.0)),
            ("PaperKind", num(paper)),
        ],
    );
    sh
}

/// a new document of this kind with JFLib's own defaults (see the module comment); a stencil
/// has no masters yet, a drawing and a template one empty page
pub fn new_document(kind: DocKind, measure: Measure) -> Document {
    let mut doc = Document::new(kind);
    for (n, v) in [("xmlns", NS_VISIO), ("xmlns:r", NS_REL), ("xml:space", "preserve")] {
        let pair = (doc.intern(n), doc.intern(v));
        doc.attrs.push(pair);
    }
    doc.settings = DocSettings {
        top_page: Some(0),
        default_text_style: Some(1),
        default_line_style: Some(1),
        default_fill_style: Some(1),
        default_guide_style: Some(3),
        ..DocSettings::default()
    };
    for (i, rgb) in COLORS.iter().enumerate() {
        let rgb = doc.intern(rgb);
        doc.colors.push(ColorEntry { ix: i as u32, rgb });
    }
    let face = (doc.intern("NameU"), doc.intern(FONT));
    doc.faces.push(FaceName { attrs: alloc::vec![face] });
    for st in styles(&mut doc) {
        let _ = doc.insert_style(st);
    }
    let the_doc = doc.intern("TheDoc");
    let (name_u, name) = (doc.intern("NameU"), doc.intern("Name"));
    let ds = doc.doc_sheet_mut();
    ds.styles = StyleRefs::all(0);
    ds.attrs = alloc::vec![(name_u, the_doc), (name, the_doc)];
    // one page: Page-1 (its own part unless the document is a stencil)
    let page_name = doc.intern("Page-1");
    let mut page = Page::new(PageHeader { id: 0, name: Some(page_name), name_u: Some(page_name), sheet_styles: StyleRefs::all(0), ..PageHeader::default() });
    page.sheet = page_sheet(&mut doc, measure);
    let _ = doc.insert_page(page);
    // the theme and the document properties
    let part = doc.intern("/visio/theme/theme1.xml");
    doc.themes.push(Theme { part, data: theme::netlab() });
    let props: [(&str, &str, &str, Vec<u8>); 2] = [
        (
            "/docProps/core.xml",
            "application/vnd.openxmlformats-package.core-properties+xml",
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
            alloc::vec::Vec::from(
                &b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"/>"[..],
            ),
        ),
        (
            "/docProps/app.xml",
            "application/vnd.openxmlformats-officedocument.extended-properties+xml",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties",
            alloc::vec::Vec::from(
                &b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>JFLib</Application></Properties>"[..],
            ),
        ),
    ];
    for (name, ct, rel, data) in props {
        let part = Part { name: doc.intern(name), content_type: doc.intern(ct), rel_type: doc.intern(rel), data };
        doc.parts.push(part);
    }
    doc
}
