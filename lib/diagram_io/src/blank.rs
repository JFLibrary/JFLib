// blank -- new documents from scratch: drawing, template, stencil (vsdx::new_document)
//
// Everything here is JFLib's own: the style sheets and the document sheet hold values chosen
// for the NetLab look, the theme is the own DrawingML theme "NetLab" (theme.rs); nothing comes
// from Visio's files.  What a file needs so that Visio opens it follows the public [MS-VSDX]:
// the document part with its root style sheet ID 0 "No Style" (every other style inherits from
// it), Colors and FaceNames with at least one entry each, a Pages part with one page whose
// PageSheet is there (a drawing's and template's page has its own part, a stencil's none), the
// theme part named by the document part, the document properties (optional, written with the
// application's name only: no user data).  And windows.xml: [MS-VSDX] calls the windows part
// unused, but desktop Visio refuses a file without it (error 271, found with the user's Visio
// on 2026-11-17); an empty Windows element with the client size is enough (windows_xml).
//
// The styles (universal names as Visio knows them, so that formulas and Visio's connector
// detection find them), with the IDs of Visio's own: Visio 16 names the styles 1, 2 and 3 of a
// file it opens Text Only, None and Normal whatever their names, and gives a style that then
// has a taken name another ("Style.4"); its new drawing has 0 No Style, 1 Text Only, 2 None,
// 3 Normal, 4 Guide, 6 Theme, and its connector tool adds 7 Connector based on Normal unless
// there is a Connector already (JD26 Visio check, 2026-11-27: with Connector at 2 and Guide at
// 3, Visio's connector tool drew in our guide's look).  Files written before keep their IDs.
//   0 No Style   the root: every line, fill, text block, character and paragraph cell with a
//                plain value -- the NetLab look without a design (Signalblau 1 pt line, light
//                blue fill, anthracite Inter 10 pt, centred text); themed cells fall back to it
//   6 Theme      based on No Style (JD11): every cell a design sets is THEMEVAL() (V "Themed"),
//                the QuickStyle cells 100 (the variant's colour 1 and style 1), the theme index
//                cells 65534 (the page's design)
//   3 Normal     based on Theme: what shapes use
//   2 None       based on Normal: no line, no fill (as Visio's)
//   7 Connector  based on Normal: the connector scheme's style 1 (line with its arrow, text),
//                no fill, the background colour behind the text; the line in the variant's
//                colour 7 (QuickStyleLineColor 106): NetLab's connector line is dk2 (slate),
//                which Visio 16 does not take from a theme's style lists -- there it draws the
//                shape's quick style colour instead (as for phClr; also a fixed srgbClr, and
//                transforms on dk1 it drops), and colour 7 is that slate in NetLab's variants
//                1 .. 3 (JD26 Visio check, 2026-11-27).  JFDiagrammer reads dk2 as dk2; the
//                designs whose connector line is dk1 look the same with any colour
//   4 Guide      based on No Style: thin light blue dashed line, no fill
//   1 Text Only  based on Normal: no line, no fill, the dark text of font style 1, with
//                Visio's contrast rule for text (QuickStyleVariation 2)
// DocumentSettings name Normal as the default text, line and fill style and Guide as the
// default guide style.  The page is A4 (metric, millimetres) or Letter (US, inches), landscape,
// and shows the design NetLab, variant 1 (design::catalog).

use alloc::vec::Vec;

use diagram_core::design::{self, catalog::NETLAB};
use diagram_core::{
    Cell, ColorEntry, DocKind, DocSettings, Document, FaceName, Formula, Page, PageHeader, Part, Row, RowKey, SecKey, Section,
    Sheet, Style, StyleHeader, StyleRefs, Theme, Unit, Value,
};

use crate::vsdx::CT_WINDOWS;
use crate::xml::{NS_REL, NS_VISIO};

/// the windows part of a document that has none: the Windows element with a client area of
/// its own size and no window in it (Visio opens its drawing window by itself)
pub(crate) fn windows_xml() -> Vec<u8> {
    let s = alloc::format!(
        "{}<Windows ClientWidth='1600' ClientHeight='900' xmlns='{NS_VISIO}' xmlns:r='{NS_REL}' xml:space='preserve'/>",
        crate::write::DECL
    );
    s.into_bytes()
}

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
            ("LineWeight", pt(1.0)),
            ("LineColor", V::T(SIGNALBLAU)),
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
            ("FillForegnd", V::T(INFO_BG)),
            ("FillBkgnd", V::T(INFO_BG)),
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

// the cells a design sets: THEMEVAL(), stored as "Themed"
const THEMED: [&str; 22] = [
    "LineWeight", "LineColor", "LinePattern", "Rounding", "LineCap", "LineColorTrans", "CompoundType", "FillForegnd", "FillBkgnd",
    "FillPattern", "ShdwForegnd", "ShdwPattern", "FillForegndTrans", "FillBkgndTrans", "ShdwForegndTrans", "ShapeShdwType",
    "ShapeShdwOffsetX", "ShapeShdwOffsetY", "ShapeShdwObliqueAngle", "ShapeShdwScaleFactor", "ShapeShdwBlur", "FillGradientEnabled",
];

const QUICK: [&str; 8] = [
    "QuickStyleLineColor", "QuickStyleFillColor", "QuickStyleShadowColor", "QuickStyleFontColor", "QuickStyleLineMatrix",
    "QuickStyleFillMatrix", "QuickStyleEffectsMatrix", "QuickStyleFontMatrix",
];

const INDEXES: [&str; 8] = [
    "ColorSchemeIndex", "EffectSchemeIndex", "ConnectorSchemeIndex", "FontSchemeIndex", "ThemeIndex", "VariationColorIndex",
    "VariationStyleIndex", "EmbellishmentIndex",
];

fn themed() -> V {
    V::F("Themed", "THEMEVAL()")
}

// the style IDs (see the module comment)
const TEXT_ONLY: u32 = 1;
const NONE: u32 = 2;
const NORMAL: u32 = 3;
const GUIDE: u32 = 4;
const THEME: u32 = 6;
const CONNECTOR: u32 = 7;

// 6 Theme (see the module comment)
fn theme_style(doc: &mut Document) -> Style {
    let mut list: Vec<(&str, V)> = THEMED.iter().map(|n| (*n, themed())).collect();
    list.extend(QUICK.iter().map(|n| (*n, num(100.0))));
    list.push(("QuickStyleType", num(0.0)));
    list.push(("QuickStyleVariation", num(0.0)));
    list.extend(INDEXES.iter().map(|n| (*n, num(65534.0))));
    let mut sh = Sheet::new();
    sh.cells = cells(doc, list);
    let character = one_row(doc, "Character", alloc::vec![("Font", themed()), ("Color", themed()), ("Style", themed())]);
    sh.sections.push(character);
    style(doc, THEME, "Theme", Some(0), sh)
}

fn styles(doc: &mut Document) -> Vec<Style> {
    let root = no_style(doc);
    let theme = theme_style(doc);
    let normal = Sheet::new();
    let mut connector = Sheet::new();
    connector.cells = cells(
        doc,
        alloc::vec![
            // variant colour 7 (see the module comment)
            ("QuickStyleLineColor", num(106.0)),
            ("QuickStyleLineMatrix", num(1.0)),
            ("QuickStyleFillMatrix", num(1.0)),
            ("QuickStyleEffectsMatrix", num(1.0)),
            ("QuickStyleFontMatrix", num(1.0)),
            ("BeginArrow", themed()),
            ("EndArrow", themed()),
            ("BeginArrowSize", themed()),
            ("EndArrowSize", themed()),
            ("FillPattern", num(0.0)),
            ("TextBkgnd", V::F(WHITE, "THEMEVAL(\"BackgroundColor\")+1")),
        ],
    );
    let mut guide = Sheet::new();
    guide.cells = cells(
        doc,
        alloc::vec![("LineWeight", pt(0.0)), ("LineColor", V::T(SIGNALBLAU_LIGHT)), ("LinePattern", num(2.0)), ("FillPattern", num(0.0))],
    );
    let mut text_only = Sheet::new();
    text_only.cells = cells(
        doc,
        alloc::vec![("LinePattern", num(0.0)), ("FillPattern", num(0.0)), ("QuickStyleFontMatrix", num(1.0)), ("QuickStyleVariation", num(2.0))],
    );
    let mut none = Sheet::new();
    none.cells = cells(doc, alloc::vec![("LinePattern", num(0.0)), ("FillPattern", num(0.0))]);
    // in the order of the IDs, as Visio writes them
    alloc::vec![
        root,
        style(doc, TEXT_ONLY, "Text Only", Some(NORMAL), text_only),
        style(doc, NONE, "None", Some(NORMAL), none),
        style(doc, NORMAL, "Normal", Some(THEME), normal),
        style(doc, GUIDE, "Guide", Some(0), guide),
        theme,
        style(doc, CONNECTOR, "Connector", Some(NORMAL), connector),
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
            // the design NetLab, variant 1
            ("ThemeIndex", num(NETLAB as f64)),
            ("ColorSchemeIndex", num(NETLAB as f64)),
            ("EffectSchemeIndex", num(NETLAB as f64)),
            ("ConnectorSchemeIndex", num(NETLAB as f64)),
            ("FontSchemeIndex", num(NETLAB as f64)),
            ("VariationColorIndex", num(0.0)),
            ("VariationStyleIndex", num(0.0)),
            ("EmbellishmentIndex", num(0.0)),
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
        default_text_style: Some(NORMAL),
        default_line_style: Some(NORMAL),
        default_fill_style: Some(NORMAL),
        default_guide_style: Some(GUIDE),
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
    doc.themes.push(Theme { part, data: design::catalog::netlab().theme_xml() });
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
    let windows = Part {
        name: doc.intern("/visio/windows.xml"),
        content_type: doc.intern(CT_WINDOWS),
        rel_type: doc.intern("http://schemas.microsoft.com/visio/2010/relationships/windows"),
        data: windows_xml(),
    };
    doc.parts.push(windows);
    doc
}
