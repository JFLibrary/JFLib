// stencil -- the stencil build tool (JD9): the text sources of a stencil into a stencil document
// and a .vssx file
//
//   stencil::build(&files, Measure::Metric) -> Result<Built, SourceError>
//   stencil::build_vssx(&files, Measure::Us) -> Result<(Built, Vec<u8>), SourceError>
//   Built { doc, stencil, file, category, measure }   the stencil document (DocKind::Stencil),
//                                   its universal name, the output stem, the category
//   built.vssx_name()               "jfd_standard_m.vssx" / "jfd_standard_u.vssx"
//   file_name(stem, Measure) / parse_file_name(name)   (JD13) the file names of JFDiagrammer's
//                                   own stencils: the prefix "jfd_" keeps them apart from
//                                   Visio's (its ARROWS_M.VSSX is not our arrows_m)
//   SourceError { file, line, message }   shown as "rectangle.master:12: unbekannter
//                                   Abschnitt [Geometrie]" (line 0: the whole file)
//
// `files(name)` gives the text of "stencil.txt", of "<stem>.master" and of an SVG file named by
// a master, all of one stencil folder (None: there is none).  The source format, and what goes
// into the file, is apps/JFDiagrammer/stencils/README.md; the app's build.rs calls this for each
// stencil and both measures.  The parser is stencil_src.rs, the SVG import stencil_svg.rs, the
// icons stencil_icon.rs.
//
// How a stencil is built, in this order:
// 1. stencil.txt and every master file are parsed and checked for the chosen measure: header
//    keys, sections, row heads, cell names (each section's own list), the five languages of
//    every language-suffixed key or cell, Width and Height as lengths or formulas (a box that
//    fits its text: TEXTHEIGHT ...; not with svg:) (a 1-D master: BeginX, BeginY, EndX, EndY as
//    lengths and Height), formulas that parse and call only functions the engine knows.  A
//    ".metric" / ".us" line replaces the line without suffix.
// 2. A new stencil (vsdx::new_document: the own styles and the NetLab theme) gets the masters in
//    one transaction: the Master attributes (Name and Prompt in the build language,
//    IsCustomName(U), IconSize 1, AlignName 2, MatchByName 0, IconUpdate 1, PatternFlags 0,
//    Hidden 0 (1 with the header key `hidden: true`: a master the Shapes panel does not show,
//    which code drops), MasterType 2, BaseID and UniqueID), the master page (PageWidth / PageHeight =
//    the shape's size, scale 1:1 in mm or in, ShapeKeywords, the JFD_ User cells), the master
//    shape with its cells, sections, text and the SVG geometry.  Constants are written as values
//    with their unit (lengths in inches with U = MM, IN ...), everything else as a formula; the
//    tool adds PinX / PinY (half the size), LocPinX = Width*0.5, LocPinY = Height*0.5.  A 1-D
//    master (one that sets BeginX ...) gets Visio's 1-D formulas instead: PinX =
//    GUARD((BeginX+EndX)/2), Width = GUARD(SQRT(...)), Angle = GUARD(ATAN2(EndY-BeginY,
//    EndX-BeginX)), LocPin = GUARD(Width*0.5) ...; its master page is BeginX+EndX wide and
//    BeginY+EndY high.
// 3. Every reference of every formula must name something that is there (a cell of the sheet
//    or of its styles, a single cell of the ShapeSheet's own list, Sheet.N of the master).  The
//    formula engine computes every formula and its values are stored: a cell the engine keeps
//    (unknown function, unknown section, cycle) or that comes out as an error is a build error
//    naming the source line.  No stale value goes into the file.  A master whose Width or
//    Height is a formula gets its master page and pin from the computed size afterwards.
// 4. Each master's icon is rendered from an instance on the stencil's page (stencil_icon.rs);
//    the instance goes again, the page stays empty.
//
// The GUIDs are name-based (SHA-1, RFC 4122 version 5, namespace of this tool): BaseID from the
// stencil's NameU, the master's NameU and the measure; UniqueID from the same and the master's
// content as built (the resolved lines, the SVG, the build language, the quick flag, the tool's
// version), so an unchanged master keeps its UniqueID and a changed one gets a new one (MS-02).
// Upper case in braces, as Visio writes them.  The same sources always give the same document
// and the same bytes.
//
// The JFD_ cells (diagram_core::instance reads them): the document sheet's User.JFD_Name (the
// title in de|en|es|it|fr) and User.JFD_Category; per master page User.JFD_Name, JFD_Prompt,
// JFD_Keywords, JFD_Text, JFD_Quick (1 for the first `quick` masters) and JFD_Tr1, JFD_Tr2 ...
// ("Section.Row.Cell|de|en|es|it|fr" with VSDX section names: Control.Row_1.Tip).
//
// Sources are files of the repository, trusted but checked all the same: no panic on any input,
// counts bounded, no recursion.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use diagram_core::formula::{self, CellPart, Engine, Keep, Outcome, SheetName, LOCKS};
use diagram_core::instance::{self, join_fields, LANGS};
use diagram_core::{
    CellAddr, DocKind, Document, EditError, MasterRef, Owner, RowKey, SecKey, ShapeKind, ShapeRef, SheetRef, StyleRefs, Text,
    Transaction, Unit, Value,
};

use crate::stencil_icon;
use crate::stencil_src::{self, err, Entry, SecKind, Value0};
use crate::stencil_svg::{self, Art, Layer, Seg};
use crate::vsdx::{self, Measure, WriteOptions};

/// a stencil built from its sources
#[derive(Clone, Debug)]
pub struct Built {
    /// the stencil (DocKind::Stencil)
    pub doc: Document,
    /// its universal name (stencil.txt `stencil`)
    pub stencil: String,
    /// the stem of the output files (stencil.txt `file`)
    pub file: String,
    /// the category in "Weitere Shapes" (stencil.txt `category`)
    pub category: String,
    /// the measure it was built for
    pub measure: Measure,
}

impl Built {
    /// the file name of the .vssx: `jfd_<file>_m.vssx` (metric) or `jfd_<file>_u.vssx` (US)
    pub fn vssx_name(&self) -> String {
        file_name(&self.file, self.measure)
    }
}

/// the prefix of the file names of JFDiagrammer's own stencils (JD13, the user's choice): a
/// template names its stencils by file name (windows.xml), and Visio opens a stencil of its own
/// of that name if it has one, so ours never share a name with Visio's (ARROWS_M.VSSX)
pub const FILE_PREFIX: &str = "jfd_";

/// the file name of the built stencil with stem `stem` (stencil.txt `file`) in measure `m`:
/// "jfd_standard_m.vssx", "jfd_standard_u.vssx"
pub fn file_name(stem: &str, m: Measure) -> String {
    format!("{FILE_PREFIX}{stem}_{}.vssx", if m == Measure::Us { "u" } else { "m" })
}

/// the stem and measure of a file name file_name gives (a path before it and ASCII case do not
/// count: "C:/x/JFD_Standard_U.VSSX" -> ("standard", Us)); None for every other name, also for
/// Visio's own stencils
pub fn parse_file_name(name: &str) -> Option<(String, Measure)> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    let rest = base.strip_prefix(FILE_PREFIX)?.strip_suffix(".vssx")?;
    let (stem, m) = if let Some(s) = rest.strip_suffix("_m") {
        (s, Measure::Metric)
    } else {
        (rest.strip_suffix("_u")?, Measure::Us)
    };
    (!stem.is_empty()).then(|| (String::from(stem), m))
}

/// why a stencil did not build: the source file, the line (from 1; 0: the whole file) and why
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceError {
    /// the file in the stencil's folder ("rectangle.master")
    pub file: String,
    /// the line, from 1 (0: the file as a whole)
    pub line: u32,
    /// what is wrong
    pub message: String,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

/// the categories of "Weitere Shapes" a stencil may name
pub const CATEGORIES: [&str; 9] = ["general", "flowchart", "business", "engineering", "network", "maps", "software", "schedule", "extras"];

// the version of the tool, part of every UniqueID: a change of what the tool writes gives every
// master a new one
const TOOL_VERSION: &str = "jd9-1";

// the namespace of the name-based GUIDs (made up for this tool)
const NAMESPACE: [u8; 16] = [0x6b, 0x1e, 0x4c, 0x2a, 0x9d, 0x3f, 0x4e, 0x57, 0xa1, 0xc8, 0x3f, 0x0b, 0x5d, 0x2e, 0x7a, 0x91];

const MAX_MASTERS: usize = 10_000;

// ---------------------------------------------------------------- what a section may hold

/// the single cells of a shape the tool knows (README: [Shape]); LOCKS and the effect families
/// (Bevel..., Glow..., Reflection..., Sketch..., Rotation...) besides
const SINGLE_CELLS: &[&str] = &[
    // transform, 1-D, text transform
    "PinX", "PinY", "Width", "Height", "LocPinX", "LocPinY", "Angle", "FlipX", "FlipY", "ResizeMode", "BeginX", "BeginY", "EndX",
    "EndY", "TxtPinX", "TxtPinY", "TxtWidth", "TxtHeight", "TxtLocPinX", "TxtLocPinY", "TxtAngle",
    // line
    "LineWeight", "LineColor", "LinePattern", "Rounding", "EndArrowSize", "BeginArrow", "EndArrow", "LineCap", "BeginArrowSize",
    "LineColorTrans", "CompoundType",
    // fill and shadow
    "FillForegnd", "FillBkgnd", "FillPattern", "ShdwForegnd", "ShdwPattern", "FillForegndTrans", "FillBkgndTrans",
    "ShdwForegndTrans", "ShapeShdwType", "ShapeShdwOffsetX", "ShapeShdwOffsetY", "ShapeShdwObliqueAngle", "ShapeShdwScaleFactor",
    "ShapeShdwBlur", "ShapeShdwShow",
    // gradients
    "FillGradientEnabled", "FillGradientDir", "FillGradientAngle", "LineGradientEnabled", "LineGradientDir", "LineGradientAngle",
    "RotateGradientWithShape", "UseGroupGradient",
    // text block
    "LeftMargin", "RightMargin", "TopMargin", "BottomMargin", "VerticalAlign", "TextBkgnd", "TextBkgndTrans", "TextDirection",
    "DefaultTabStop",
    // miscellaneous, events, glue
    "NoObjHandles", "NonPrinting", "NoCtlHandles", "NoAlignBox", "UpdateAlignBox", "HideText", "DynFeedback", "GlueType",
    "WalkPreference", "BegTrigger", "EndTrigger", "ObjType", "Comment", "IsDropSource", "NoLiveDynamics", "LocalizeMerge",
    "NoProofing", "Calendar", "LangID", "ShapeKeywords", "DropOnPageScale", "EventDblClick", "EventXFMod", "EventDrop",
    "EventMultiDrop", "TheData", "TheText",
    // layout
    "ShapePermeableX", "ShapePermeableY", "ShapePermeablePlace", "ShapeFixedCode", "ShapePlowCode", "ShapeRouteStyle",
    "ShapePlaceDepth", "ShapePlaceFlip", "ShapePlaceStyle", "ConFixedCode", "ConLineJumpCode", "ConLineJumpStyle",
    "ConLineJumpDirX", "ConLineJumpDirY", "ConLineRouteExt", "ShapeSplit", "ShapeSplittable", "DisplayLevel", "Relationships",
    // group, image, layers, alignment
    "SelectMode", "DisplayMode", "IsDropTarget", "IsSnapTarget", "IsTextEditTarget", "DontMoveChildren", "ImgOffsetX",
    "ImgOffsetY", "ImgWidth", "ImgHeight", "Brightness", "Contrast", "Gamma", "Transparency", "Blur", "Sharpen", "Denoise",
    "LayerMember", "AlignLeft", "AlignCenter", "AlignRight", "AlignTop", "AlignMiddle", "AlignBottom",
    // theme
    "ThemeIndex", "ColorSchemeIndex", "EffectSchemeIndex", "ConnectorSchemeIndex", "FontSchemeIndex", "VariationColorIndex",
    "VariationStyleIndex", "EmbellishmentIndex", "QuickStyleLineColor", "QuickStyleFillColor", "QuickStyleShadowColor",
    "QuickStyleFontColor", "QuickStyleLineMatrix", "QuickStyleFillMatrix", "QuickStyleEffectsMatrix", "QuickStyleFontMatrix",
    "QuickStyleType", "QuickStyleVariation",
    // effects
    "SoftEdgesSize", "Perspective", "DistanceFromGround", "KeepTextFlat",
];
const EFFECT_FAMILIES: &[&str] = &["Bevel", "Glow", "Reflection", "Sketch", "Rotation"];
// the page sheet's cells formulas may name (ThePage!PageWidth ...)
const PAGE_CELLS: &[&str] = &["PageWidth", "PageHeight", "PageScale", "DrawingScale", "DrawingSizeType", "DrawingScaleType", "ShdwOffsetX", "ShdwOffsetY"];

// a cell [Shape] may set
pub(crate) fn shape_cell_known(name: &str) -> bool {
    SINGLE_CELLS.contains(&name) || LOCKS.contains(&name) || EFFECT_FAMILIES.iter().any(|p| name.starts_with(p) && name.len() > p.len())
}

// a single cell a formula may name without a sheet having it (Visio's default)
fn single_cell_known(name: &str) -> bool {
    shape_cell_known(name) || PAGE_CELLS.contains(&name)
}

const GEO_SECTION_CELLS: &[&str] = &["NoFill", "NoLine", "NoShow", "NoSnap", "NoQuickDrag"];
const GEO_ROW_CELLS: &[&str] = &["X", "Y", "A", "B", "C", "D", "E"];

// the cells of a row of a section (by its VSDX name)
pub(crate) fn row_cells(model: &str) -> &'static [&'static str] {
    match model {
        "Connection" => &["X", "Y", "DirX", "DirY", "Type", "AutoGen", "Prompt", "A", "B", "C", "D"],
        "Control" => &["X", "Y", "XDyn", "YDyn", "XCon", "YCon", "CanGlue", "Tip"],
        "User" => &["Value", "Prompt"],
        "Property" => &["Label", "Prompt", "Type", "Format", "Value", "SortKey", "Invisible", "Verify", "DataLinked", "LangID", "Calendar"],
        "Character" => &[
            "Font", "Color", "Style", "Case", "Pos", "FontScale", "Size", "DblUnderline", "Overline", "Strikethru",
            "DoubleStrikethrough", "Letterspace", "ColorTrans", "AsianFont", "ComplexScriptFont", "ComplexScriptSize", "LangID",
            "LocalizeFont", "RTLText", "UseVertical", "Perpendicular",
        ],
        "Paragraph" => &[
            "IndFirst", "IndLeft", "IndRight", "SpLine", "SpBefore", "SpAfter", "HorzAlign", "Bullet", "BulletStr", "BulletFont",
            "BulletFontSize", "TextPosAfterBullet", "Flags",
        ],
        "Actions" => &["Menu", "Action", "Checked", "Disabled", "ReadOnly", "Invisible", "BeginGroup", "TagName", "ButtonFace", "SortKey", "FlyoutChild"],
        "Hyperlink" => &["Description", "Address", "SubAddress", "ExtraInfo", "Frame", "NewWindow", "Default", "Invisible", "SortKey"],
        "Scratch" => &["X", "Y", "A", "B", "C", "D"],
        _ => &[],
    }
}

// ---------------------------------------------------------------- resolved sources

// a cell as the builder writes it
#[derive(Clone, Debug)]
enum CellVal {
    Plain(Value0),
    // a language-suffixed cell: its text in the five languages
    Text([String; 5]),
}

#[derive(Clone, Debug)]
struct Cell0 {
    name: String,
    line: u32,
    val: CellVal,
    // the included file it comes from (`include:`), else the master's
    file: Option<String>,
}

#[derive(Clone, Debug)]
struct Row0 {
    head: String,
    line: u32,
    cells: Vec<Cell0>,
}

#[derive(Clone, Debug)]
struct Sec0 {
    model: &'static str,
    kind: SecKind,
    line: u32,
    cells: Vec<Cell0>,
    rows: Vec<Row0>,
}

struct StencilInfo {
    stencil: String,
    file: String,
    names: [String; 5],
    category: String,
    masters: Vec<String>,
    masters_line: u32,
    quick: usize,
    lang: usize,
}

struct MasterInfo {
    file: String,
    // the line of `master:`
    line: u32,
    name_u: String,
    names: [String; 5],
    prompts: [String; 5],
    keywords: Option<[String; 5]>,
    text: Option<[String; 5]>,
    // line, fill, text style by NameU with the line naming it
    styles: [Option<(String, u32)>; 3],
    svg: Option<(Art, u32)>,
    // `hidden: true`: the master is in the file but not shown in the Shapes panel (Hidden="1";
    // dropped by code, e.g. the lists of a cross-functional flowchart)
    hidden: bool,
    // (JD13) `scale: 1:N`: drawing units per page unit of the master page (1: no scale)
    scale: f64,
    // the size of the master page: the shape's Width x Height (2-D), BeginX+EndX x BeginY+EndY
    // (1-D), `size:` for a formula (JD13); None: Width or Height is a formula, the page takes the
    // size the engine computes
    width: Option<f64>,
    height: Option<f64>,
    // (1-D) the master sets BeginX, BeginY, EndX, EndY: PinX, Width, Angle ... follow the ends
    one_d: bool,
    shape: Vec<Cell0>,
    sections: Vec<Sec0>,
    // (JD26) the subshapes ([Part NAME]): the master is a group
    parts: Vec<PartInfo>,
    // the content for the UniqueID
    canon: String,
}

// (JD26) a subshape of a group master ([Part NAME], README: Gruppen aus Teilen)
struct PartInfo {
    name: String,
    line: u32,
    text: Option<[String; 5]>,
    // line, fill, text style by NameU with the line naming it (None: Normal)
    styles: [Option<(String, u32)>; 3],
    shape: Vec<Cell0>,
    sections: Vec<Sec0>,
}

// the values of a key in five languages (`required`: an error when there is none)
pub(crate) fn five(file: &str, es: &[&Entry], key: &str, required: bool, line: u32) -> Result<Option<[String; 5]>, SourceError> {
    let mut out: [Option<String>; 5] = Default::default();
    let mut first = None;
    for e in es.iter().filter(|e| e.name == key) {
        let Some(l) = e.lang else {
            return Err(err(file, e.line, format!("{key} braucht eine Sprachendung ({key}.de, {key}.en, {key}.es, {key}.it, {key}.fr)")));
        };
        first.get_or_insert(e.line);
        out[l] = Some(e.value.clone());
    }
    let Some(first) = first else {
        return if required { Err(err(file, line, format!("{key}.de … {key}.fr fehlen"))) } else { Ok(None) };
    };
    if let Some(i) = out.iter().position(Option::is_none) {
        return Err(err(file, first, format!("{key}.{} fehlt", LANGS[i])));
    }
    Ok(Some(core::array::from_fn(|i| out[i].take().unwrap_or_default())))
}

// the one entry of a key without language
pub(crate) fn single<'e>(file: &str, es: &[&'e Entry], key: &str) -> Result<Option<&'e Entry>, SourceError> {
    match es.iter().find(|e| e.name == key) {
        Some(e) if e.lang.is_some() => Err(err(file, e.line, format!("{key} hat keine Sprachendung"))),
        found => Ok(found.copied()),
    }
}

pub(crate) fn keys_known(file: &str, es: &[&Entry], known: &[&str]) -> Result<(), SourceError> {
    match es.iter().find(|e| !known.contains(&e.name.as_str())) {
        Some(e) => Err(err(file, e.line, format!("unbekannter Schlüssel {} (bekannt: {})", e.name, known.join(", ")))),
        None => Ok(()),
    }
}

fn stencil_info(text: &str, m: Measure) -> Result<StencilInfo, SourceError> {
    const F: &str = "stencil.txt";
    let src = stencil_src::parse(F, text, false)?;
    let es = stencil_src::resolve(F, &src.header, m)?;
    keys_known(F, &es, &["stencil", "file", "name", "category", "masters", "quick", "language"])?;
    let need = |key: &str| -> Result<&Entry, SourceError> {
        match single(F, &es, key)? {
            Some(e) if !e.value.is_empty() => Ok(e),
            Some(e) => Err(err(F, e.line, format!("{key}: Wert fehlt"))),
            None => Err(err(F, 1, format!("{key}: fehlt"))),
        }
    };
    let stencil = need("stencil")?.value.clone();
    let fe = need("file")?;
    if !fe.value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(err(F, fe.line, format!("file: „{}“ ist kein Dateistamm (Buchstaben, Ziffern, _ und -)", fe.value)));
    }
    let names = five(F, &es, "name", true, 1)?.unwrap_or_default();
    let ce = need("category")?;
    if !CATEGORIES.contains(&ce.value.as_str()) {
        return Err(err(F, ce.line, format!("category: unbekannte Kategorie {} (bekannt: {})", ce.value, CATEGORIES.join(", "))));
    }
    let me = need("masters")?;
    let mut masters: Vec<String> = Vec::new();
    for part in me.value.split(',') {
        let p = part.trim();
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(err(F, me.line, format!("masters: „{p}“ ist kein Dateistamm")));
        }
        if masters.iter().any(|x| x == p) {
            return Err(err(F, me.line, format!("masters: {p} doppelt")));
        }
        if masters.len() >= MAX_MASTERS {
            return Err(err(F, me.line, "masters: zu viele Master"));
        }
        masters.push(String::from(p));
    }
    let quick = match single(F, &es, "quick")? {
        None => 0,
        Some(e) => match e.value.parse::<usize>() {
            Ok(q) if q <= masters.len() => q,
            _ => return Err(err(F, e.line, format!("quick: „{}“ ist keine Zahl von 0 bis {}", e.value, masters.len()))),
        },
    };
    let lang = match single(F, &es, "language")? {
        None => 0,
        Some(e) => instance::lang_index(&e.value).ok_or_else(|| err(F, e.line, format!("language: „{}“ ist keine der Sprachen {}", e.value, LANGS.join(", "))))?,
    };
    Ok(StencilInfo { stencil, file: fe.value.clone(), names, category: ce.value.clone(), masters, masters_line: me.line, quick, lang })
}

// the cells of one scope (single cells, a section's own cells, a row) for variant m
fn cells_of(file: &str, entries: &[Entry], m: Measure, allowed: &dyn Fn(&str) -> bool, place: &str, langs: bool) -> Result<Vec<Cell0>, SourceError> {
    let resolved = stencil_src::resolve(file, entries, m)?;
    let mut out: Vec<(Cell0, u8)> = Vec::new();
    let mut at: BTreeMap<&str, usize> = BTreeMap::new();
    let master = file;
    for e in resolved {
        // an included entry names its own file (JD13, `include:`)
        let file = e.file.as_deref().unwrap_or(master);
        if !allowed(&e.name) {
            return Err(err(file, e.line, format!("unbekannte Zelle {} in {place}", e.name)));
        }
        if !at.contains_key(e.name.as_str()) {
            at.insert(&e.name, out.len());
        }
        match (at.get(e.name.as_str()).and_then(|&i| out.get_mut(i)), e.lang) {
            (Some((Cell0 { val: CellVal::Text(t), .. }, have)), Some(l)) => {
                t[l] = stencil_src::unquote(&e.value);
                *have |= 1 << l;
            }
            (Some(_), _) => return Err(err(file, e.line, format!("{}: mit und ohne Sprachendung", e.name))),
            (None, Some(l)) => {
                if !langs {
                    return Err(err(file, e.line, format!("{}: Sprachendungen gibt es in {place} nicht", stencil_src::show(e))));
                }
                let mut t: [String; 5] = Default::default();
                t[l] = stencil_src::unquote(&e.value);
                out.push((Cell0 { name: e.name.clone(), line: e.line, val: CellVal::Text(t), file: e.file.clone() }, 1 << l));
            }
            (None, None) => {
                let v = stencil_src::classify(&e.value);
                if let Value0::Formula(f) = &v {
                    check_syntax(file, e.line, &e.name, f)?;
                }
                out.push((Cell0 { name: e.name.clone(), line: e.line, val: CellVal::Plain(v), file: e.file.clone() }, 0));
            }
        }
    }
    for (c, have) in &out {
        if matches!(c.val, CellVal::Text(_)) && *have != 0b1_1111 {
            let missing = (0..5).find(|i| have & (1 << i) == 0).unwrap_or(0);
            return Err(err(c.file.as_deref().unwrap_or(file), c.line, format!("{}: Sprache {} fehlt", c.name, LANGS[missing])));
        }
    }
    Ok(out.into_iter().map(|(c, _)| c).collect())
}

// a formula must parse and call only functions the engine knows
fn check_syntax(file: &str, line: u32, cell: &str, f: &str) -> Result<(), SourceError> {
    match formula::parse(f) {
        Err(e) => Err(err(file, line, format!("{cell}: Formel nicht lesbar ab Zeichen {}: {f}", e.pos + 1))),
        Ok(p) => match p.unknown().next() {
            Some(name) => Err(err(file, line, format!("{cell}: unbekannte Funktion {name} in {f}"))),
            None => Ok(()),
        },
    }
}

fn master_info(file: &str, text: &str, m: Measure, files: &dyn Fn(&str) -> Option<String>) -> Result<MasterInfo, SourceError> {
    let mut src = stencil_src::parse(file, text, true)?;
    let first = src.header.first().ok_or_else(|| err(file, 1, "die erste Kopfzeile muss master: NameU sein"))?;
    if first.name != "master" || first.lang.is_some() || first.variant.is_some() {
        return Err(err(file, first.line, "die erste Kopfzeile muss master: NameU sein"));
    }
    let line = first.line;
    // (JD13) `include: a.inc, b.inc`: sections shared by many masters, from other files of the
    // stencil folder (README); their text is part of the UniqueID
    let include = {
        let es = stencil_src::resolve(file, &src.header, m)?;
        single(file, &es, "include")?.map(|e| (e.value.clone(), e.line))
    };
    let mut included = String::new();
    if let Some((list, at)) = include {
        for name in list.split(',').map(str::trim) {
            if name.is_empty() {
                return Err(err(file, at, "include: leerer Dateiname"));
            }
            let t = files(name).ok_or_else(|| err(file, at, format!("die eingebundene Datei {name} fehlt")))?;
            let inc = stencil_src::parse(name, &t, true)?;
            stencil_src::merge_include(&mut src, name, inc)?;
            included.push_str(&format!("include\u{1f}{name}\u{1f}{t}\u{1e}"));
        }
    }
    let es = stencil_src::resolve(file, &src.header, m)?;
    keys_known(file, &es, &["master", "name", "prompt", "keywords", "text", "style", "line_style", "fill_style", "text_style", "svg", "hidden", "include", "scale", "size"])?;
    // (JD13) `scale: 1:20`: the master page's drawing scale (shapes drawn to scale, the rack)
    let scale = match single(file, &es, "scale")? {
        None => 1.0,
        Some(e) => parse_scale(&e.value).ok_or_else(|| err(file, e.line, format!("scale: „{}“ ist kein Maßstab 1:N (z. B. 1:20)", e.value)))?,
    };
    let name_u = single(file, &es, "master")?.map(|e| e.value.clone()).unwrap_or_default();
    if name_u.is_empty() {
        return Err(err(file, line, "master: NameU fehlt"));
    }
    let names = five(file, &es, "name", true, line)?.unwrap_or_default();
    let prompts = five(file, &es, "prompt", true, line)?.unwrap_or_default();
    let keywords = five(file, &es, "keywords", false, line)?;
    let text = five(file, &es, "text", false, line)?;
    let hidden = match single(file, &es, "hidden")? {
        None => false,
        Some(e) if e.value.eq_ignore_ascii_case("true") => true,
        Some(e) if e.value.eq_ignore_ascii_case("false") => false,
        Some(e) => return Err(err(file, e.line, format!("hidden: „{}“ ist weder true noch false", e.value))),
    };
    let style_of = |key: &str| -> Result<Option<(String, u32)>, SourceError> { Ok(single(file, &es, key)?.map(|e| (e.value.clone(), e.line))) };
    let all = style_of("style")?;
    let styles = [style_of("line_style")?.or(all.clone()), style_of("fill_style")?.or(all.clone()), style_of("text_style")?.or(all)];
    let mut canon = String::new();
    for e in &es {
        canon.push_str(&format!("{}\u{1f}{}\u{1f}{}\u{1e}", e.name, e.lang.map_or("", |l| LANGS[l]), e.value));
    }
    canon.push_str(&included);
    let svg = match single(file, &es, "svg")? {
        None => None,
        Some(e) => {
            let t = files(&e.value).ok_or_else(|| err(file, e.line, format!("die SVG-Datei {} fehlt", e.value)))?;
            let art = stencil_svg::import(&e.value, &t)?;
            if art.layers.is_empty() {
                return Err(err(&e.value, 0, "das SVG zeichnet nichts"));
            }
            // (JD13) the layers in document order: a new UniqueID for every SVG master
            canon.push_str("svg-in-order\u{1f}");
            canon.push_str(&t);
            canon.push('\u{1e}');
            Some((art, e.line))
        }
    };
    let (shape, sections) = sections_of(file, src.sections.iter().filter(|s| s.part == 0), m, true, &mut canon)?;
    canon_cells(&mut canon, &shape);
    // (JD26) the parts: subshapes with their own header lines (text, styles) and sections
    let mut parts = Vec::new();
    for (k, p) in src.parts.iter().enumerate() {
        let pes = stencil_src::resolve(file, &p.header, m)?;
        keys_known(file, &pes, &["text", "style", "line_style", "fill_style", "text_style"])?;
        canon.push_str(&format!("[Part {}]\u{1e}", p.name));
        for e in &pes {
            canon.push_str(&format!("{}\u{1f}{}\u{1f}{}\u{1e}", e.name, e.lang.map_or("", |l| LANGS[l]), e.value));
        }
        let ptext = five(file, &pes, "text", false, p.line)?;
        let pstyle = |key: &str| -> Result<Option<(String, u32)>, SourceError> { Ok(single(file, &pes, key)?.map(|e| (e.value.clone(), e.line))) };
        let pall = pstyle("style")?;
        let pstyles = [pstyle("line_style")?.or(pall.clone()), pstyle("fill_style")?.or(pall.clone()), pstyle("text_style")?.or(pall)];
        let (pshape, psections) = sections_of(file, src.sections.iter().filter(|s| s.part == k + 1), m, false, &mut canon)?;
        canon_cells(&mut canon, &pshape);
        parts.push(PartInfo { name: p.name.clone(), line: p.line, text: ptext, styles: pstyles, shape: pshape, sections: psections });
    }
    // (JD13) `size: 482.6 mm x 88.9 mm`: the size of a master whose Width or Height is a formula,
    // also one drawn from an SVG (the rack: height from its height units); it sizes the master page
    // and the pin
    let size = match single(file, &es, "size")? {
        None => None,
        Some(e) => Some(parse_size(&e.value).ok_or_else(|| err(file, e.line, format!("size: „{}“ ist keine Größe Breite x Höhe (z. B. 482.6 mm x 88.9 mm)", e.value)))?),
    };
    let one_d = shape.iter().any(|c| ENDS.contains(&c.name.as_str()));
    let (width, height) = if one_d {
        let (w, h) = one_d_page(file, line, &shape)?;
        (Some(w), Some(h))
    } else {
        let dim = |n: &str, ix: usize| -> Result<Option<f64>, SourceError> {
            let formula = shape.iter().any(|c| c.name == n && matches!(c.val, CellVal::Plain(Value0::Formula(_))));
            match size {
                Some(s) if formula => Ok(Some(s[ix])),
                _ => length(file, line, &shape, n, svg.is_none()),
            }
        };
        (dim("Width", 0)?, dim("Height", 1)?)
    };
    if let Some(p) = parts.first() {
        if svg.is_some() {
            return Err(err(file, p.line, "[Part] und svg: zusammen gibt es nicht: ein Master ist eine Gruppe aus Teilen oder aus einer Grafik"));
        }
        if one_d {
            return Err(err(file, p.line, "ein 1D-Master hat keine Teile ([Part])"));
        }
    }
    Ok(MasterInfo { file: String::from(file), line, name_u, names, prompts, keywords, text, styles, svg, hidden, scale, width, height, one_d, shape, sections, parts, canon })
}

// the sections of the master shape or of one part: the single cells of [Shape] and the other
// sections; their content goes into `canon` (the UniqueID).  `langs`: language-suffixed cells
// are allowed (the master shape's, translated through JFD_Tr; a part has only its text)
fn sections_of<'s>(file: &str, secs: impl Iterator<Item = &'s stencil_src::SrcSection>, m: Measure, langs: bool, canon: &mut String) -> Result<(Vec<Cell0>, Vec<Sec0>), SourceError> {
    let mut shape = Vec::new();
    let mut sections = Vec::new();
    for s in secs {
        let place = format!("[{}]", s.written);
        let (cells, rows) = match s.kind {
            SecKind::Shape => {
                let cells = cells_of(file, &s.cells, m, &shape_cell_known, &place, langs)?;
                shape.extend(cells);
                canon.push_str("[Shape]\u{1e}");
                (Vec::new(), Vec::new())
            }
            SecKind::Geometry => {
                let cells = cells_of(file, &s.cells, m, &|n| GEO_SECTION_CELLS.contains(&n), &place, false)?;
                let mut rows = Vec::new();
                for r in &s.rows {
                    rows.push(Row0 { head: r.head.clone(), line: r.line, cells: cells_of(file, &r.cells, m, &|n| GEO_ROW_CELLS.contains(&n), &place, false)? });
                }
                (cells, rows)
            }
            SecKind::Indexed | SecKind::Named => {
                if let Some(e) = s.cells.first() {
                    return Err(err(file, e.line, format!("{place} hat keine eigenen Zellen ({} = …): Zeilen beginnen mit {}", e.name, if s.kind == SecKind::Named { "Name:" } else { "Row" })));
                }
                let allowed = row_cells(s.model);
                let mut rows = Vec::new();
                for r in &s.rows {
                    rows.push(Row0 { head: r.head.clone(), line: r.line, cells: cells_of(file, &r.cells, m, &|n| allowed.contains(&n), &place, langs)? });
                }
                (Vec::new(), rows)
            }
        };
        if s.kind != SecKind::Shape {
            canon.push_str(&format!("[{}]\u{1e}", s.model));
            canon_cells(canon, &cells);
            for r in &rows {
                canon.push_str(&format!("{}:\u{1e}", r.head));
                canon_cells(canon, &r.cells);
            }
            sections.push(Sec0 { model: s.model, kind: s.kind, line: s.line, cells, rows });
        }
    }
    Ok((shape, sections))
}

// the cells that make a master 1-D (README: 1D-Shapes)
const ENDS: [&str; 4] = ["BeginX", "BeginY", "EndX", "EndY"];

// a length constant of [Shape]: Some(value) when the cell is one (no unit: inches), None when it
// is something else; Err when it is missing
fn length_const(file: &str, line: u32, shape: &[Cell0], n: &str, example: &str) -> Result<(Option<f64>, u32), SourceError> {
    match shape.iter().find(|c| c.name == n) {
        None => Err(err(file, line, format!("{n} fehlt ([Shape] {n} = …, z. B. {example})"))),
        Some(Cell0 { val: CellVal::Plain(Value0::Num(x, u)), line, .. }) if u.is_none_or(|u| u.dimension() == diagram_core::Dimension::Length) => Ok((Some(*x), *line)),
        Some(c) => Ok((None, c.line)),
    }
}

// Width or Height of a 2-D master: a positive length constant, or (`formula`: the master has no
// SVG drawing) a formula such as GUARD(MAX(10 mm, TEXTHEIGHT(TheText,Width))) -> None
fn length(file: &str, line: u32, shape: &[Cell0], n: &str, formula: bool) -> Result<Option<f64>, SourceError> {
    let is_formula = shape.iter().any(|c| c.name == n && matches!(c.val, CellVal::Plain(Value0::Formula(_))));
    match length_const(file, line, shape, n, "40 mm")? {
        (Some(x), _) if x > 0.0 => Ok(Some(x)),
        (None, _) if formula && is_formula => Ok(None),
        (_, at) => Err(err(file, at, format!("{n} muss eine positive Länge sein (z. B. 40 mm) oder eine Formel (nicht mit svg:)"))),
    }
}

// a 1-D master: BeginX, BeginY, EndX, EndY are length constants from 0 (the ends on the master
// page), Height is given (a length from 0, or a formula); the master page is BeginX+EndX wide
// and BeginY+EndY high (the ends lie as far from its edges as from the other end's edges)
fn one_d_page(file: &str, line: u32, shape: &[Cell0]) -> Result<(f64, f64), SourceError> {
    let mut e = [0.0; 4];
    for (i, n) in ENDS.iter().enumerate() {
        e[i] = match length_const(file, line, shape, n, "0 mm; ein 1D-Master braucht BeginX, BeginY, EndX und EndY")? {
            (Some(x), _) if x >= 0.0 => x,
            (_, at) => return Err(err(file, at, format!("{n} muss eine Länge ab 0 sein (eine Konstante wie 40 mm): das Ende auf der Masterseite"))),
        };
    }
    match length_const(file, line, shape, "Height", "10 mm, 0 mm für eine Linie, oder eine Formel wie GUARD(EndY-BeginY)")? {
        (Some(x), at) if x < 0.0 => return Err(err(file, at, "Height muss eine Länge ab 0 oder eine Formel sein")),
        _ => {}
    }
    if (e[0] - e[2]).abs() < 1e-12 && (e[1] - e[3]).abs() < 1e-12 {
        return Err(err(file, line, "BeginX/BeginY und EndX/EndY liegen aufeinander"));
    }
    let (w, h) = (e[0] + e[2], e[1] + e[3]);
    if w <= 0.0 || h <= 0.0 {
        return Err(err(file, line, "die Masterseite (BeginX+EndX mal BeginY+EndY) wäre leer: die Enden brauchen Abstand zum Rand (z. B. BeginY = EndY = 5 mm)"));
    }
    Ok((w, h))
}

// "482.6 mm x 88.9 mm" -> both lengths (internal units)
fn parse_size(s: &str) -> Option<[f64; 2]> {
    let (a, b) = s.split_once(['x', '×'])?;
    let len = |t: &str| match stencil_src::classify(t) {
        Value0::Num(x, Some(u)) if x > 0.0 && u.dimension() == diagram_core::Dimension::Length => Some(x),
        _ => None,
    };
    Some([len(a)?, len(b)?])
}

// "1:20" -> 20 (drawing units per page unit); "2:1" -> 0.5
fn parse_scale(s: &str) -> Option<f64> {
    let (a, b) = s.split_once(':')?;
    let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a > 0.0 && b > 0.0 && a.is_finite() && b.is_finite()).then_some(b / a)
}

fn canon_cells(canon: &mut String, cells: &[Cell0]) {
    for c in cells {
        match &c.val {
            CellVal::Plain(v) => canon.push_str(&format!("{}={:?}\u{1e}", c.name, v)),
            CellVal::Text(t) => canon.push_str(&format!("{}={}\u{1e}", c.name, join_fields(&[&t[0], &t[1], &t[2], &t[3], &t[4]]))),
        }
    }
}

// ---------------------------------------------------------------- GUIDs

/// SHA-1 (FIPS 180-4) of a message: the name-based GUIDs
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// a name-based GUID (RFC 4122 version 5 in this tool's namespace) of the parts, as Visio
/// writes GUIDs: "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}"
fn guid(parts: &[&str]) -> String {
    let mut data = NAMESPACE.to_vec();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            data.push(0x1f);
        }
        data.extend_from_slice(p.as_bytes());
    }
    let h = sha1(&data);
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let mut s = String::from("{");
    for (i, x) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        s.push_str(&format!("{x:02X}"));
    }
    s.push('}');
    s
}

fn measure_name(m: Measure) -> &'static str {
    if m == Measure::Us { "us" } else { "metric" }
}

// ---------------------------------------------------------------- writing the masters

// a formula cell written, for the checks after writing
struct FormulaAt {
    sheet: SheetRef,
    addr: CellAddr,
    master: usize,
    line: u32,
    text: String,
    // the included file of its line (`include:`), else the master's
    file: Option<String>,
}

// what writing a master collects
struct Sink<'a> {
    file: &'a str,
    master: usize,
    lang: usize,
    formulas: &'a mut Vec<FormulaAt>,
    // translated cells: "Section.Row.Cell" and the five texts
    tr: Vec<(String, [String; 5])>,
    // the included file of the cell being written (`include:`), else None
    cur: Option<String>,
    // (JD26) a group master of parts: the IDs of Sheet.1 (the master shape), Sheet.2 (its first
    // part) ... as the formulas name them; empty for every other master
    ids: Vec<u32>,
}

impl Sink<'_> {
    fn fail(&self, line: u32, what: &str, e: EditError) -> SourceError {
        err(self.cur.as_deref().unwrap_or(self.file), line, format!("{what} lässt sich nicht schreiben ({e:?})"))
    }

    fn constant(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, v: Value, unit: Option<Unit>, line: u32) -> Result<(), SourceError> {
        tx.set_constant(sheet, addr, v, unit).map_err(|e| self.fail(line, tx.doc().str(addr.cell), e))
    }

    fn text(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, t: &str, unit: Option<Unit>, line: u32) -> Result<(), SourceError> {
        let s = tx.intern(t);
        self.constant(tx, sheet, addr, Value::Text(s), unit, line)
    }

    fn formula(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, f: &str, line: u32) -> Result<(), SourceError> {
        let owned;
        let f = if self.ids.is_empty() {
            f
        } else {
            owned = sheet_ids(f, &self.ids).map_err(|m| err(self.cur.as_deref().unwrap_or(self.file), line, format!("{m} in „{f}“")))?;
            owned.as_str()
        };
        tx.set_formula(sheet, addr, f).map_err(|e| self.fail(line, tx.doc().str(addr.cell), e))?;
        self.formulas.push(FormulaAt { sheet, addr: *addr, master: self.master, line, text: String::from(f), file: self.cur.clone() });
        Ok(())
    }

    // a source cell; `tr`: its "Section.Row.Cell" when it may be translated
    fn put(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, c: &Cell0, tr: Option<String>) -> Result<(), SourceError> {
        self.cur.clone_from(&c.file);
        let r = self.put_cell(tx, sheet, addr, c, tr);
        self.cur = None;
        r
    }

    fn put_cell(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, c: &Cell0, tr: Option<String>) -> Result<(), SourceError> {
        match &c.val {
            CellVal::Text(t) => {
                self.text(tx, sheet, addr, &t[self.lang], Some(Unit::Str), c.line)?;
                if let Some(r) = tr {
                    self.tr.push((r, t.clone()));
                }
                Ok(())
            }
            CellVal::Plain(v) => match v {
                Value0::Num(x, u) => self.constant(tx, sheet, addr, Value::Num(*x), *u, c.line),
                Value0::Bool(b) => self.constant(tx, sheet, addr, Value::Num(if *b { 1.0 } else { 0.0 }), Some(Unit::Bool), c.line),
                Value0::Str(s) => self.text(tx, sheet, addr, s, Some(Unit::Str), c.line),
                Value0::Color(s) => self.text(tx, sheet, addr, s, None, c.line),
                Value0::Formula(f) => self.formula(tx, sheet, addr, f, c.line),
            },
        }
    }
}

fn single_addr(tx: &mut Transaction, name: &str) -> CellAddr {
    CellAddr::single(tx.intern(name))
}

#[cfg(test)]
pub(crate) fn sheet_ids_for_test(f: &str, ids: &[u32]) -> Result<String, String> {
    sheet_ids(f, ids)
}

// (JD26) a group master's formula with its logical sheet numbers (Sheet.1! the master shape,
// Sheet.2! its first part ..., README: Gruppen aus Teilen) as the shapes' IDs; strings stay
fn sheet_ids(f: &str, ids: &[u32]) -> Result<String, String> {
    let b = f.as_bytes();
    let mut out = String::with_capacity(f.len() + 8);
    let (mut i, mut start, mut in_str) = (0, 0, false);
    while i < b.len() {
        if b[i] == b'"' {
            in_str = !in_str;
            i += 1;
            continue;
        }
        let before = i.checked_sub(1).map(|j| b[j]);
        let word_start = !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.');
        if !in_str && word_start && f[i..].starts_with("Sheet.") {
            let digits = f[i + 6..].bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && b.get(i + 6 + digits) == Some(&b'!') {
                let n: usize = f[i + 6..i + 6 + digits].parse().unwrap_or(0);
                let Some(id) = n.checked_sub(1).and_then(|k| ids.get(k)) else {
                    return Err(format!("Sheet.{n}! gibt es nicht (Sheet.1 ist der Master, Sheet.2 bis Sheet.{} seine Teile)", ids.len()));
                };
                out.push_str(&f[start..i]);
                out.push_str(&format!("Sheet.{id}!"));
                i += 7 + digits;
                start = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&f[start..]);
    Ok(out)
}

// a number in a formula: at most nine decimals, no "-0"
fn frac(x: f64) -> String {
    let r = formula::math::round(x * 1e9) / 1e9;
    if r == 0.0 || !r.is_finite() { String::from("0") } else { format!("{r}") }
}

fn style_id(tx: &Transaction, file: &str, s: &Option<(String, u32)>, line: u32) -> Result<u32, SourceError> {
    let (name, at) = match s {
        Some((n, l)) => (n.as_str(), *l),
        None => ("Normal", line),
    };
    let r = tx.doc().style_by_name_u(name).ok_or_else(|| err(file, at, format!("Formatvorlage „{name}“ gibt es nicht (No Style, Normal, Connector, Guide, Text Only)")))?;
    Ok(tx.doc().style(r).map_or(0, |st| st.header.id))
}

fn emit_master(tx: &mut Transaction, st: &StencilInfo, mi: &MasterInfo, ix: usize, measure: Measure, formulas: &mut Vec<FormulaAt>) -> Result<MasterRef, SourceError> {
    let lang = st.lang;
    let file = mi.file.as_str();
    let line = mi.line;
    let mut sink = Sink { file, master: ix, lang, formulas, tr: Vec::new(), cur: None, ids: Vec::new() };
    let quick = ix < st.quick;
    // the master and its attributes
    let mref = tx.add_master(&mi.names[lang], &mi.name_u).map_err(|e| sink.fail(line, "der Master", e))?;
    let base = guid(&["base", &st.stencil, &mi.name_u, measure_name(measure)]);
    let unique = guid(&["unique", &st.stencil, &mi.name_u, measure_name(measure), LANGS[lang], if quick { "quick" } else { "" }, TOOL_VERSION, &mi.canon]);
    let mut h = tx.doc().master(mref).map(|m| m.header.clone()).unwrap_or_default();
    h.is_custom_name = Some(true);
    h.is_custom_name_u = Some(true);
    h.base_id = Some(tx.intern(&base));
    h.unique_id = Some(tx.intern(&unique));
    h.sheet_styles = StyleRefs::all(0);
    let attrs = [
        ("Prompt", mi.prompts[lang].as_str()),
        ("IconSize", "1"),
        ("AlignName", "2"),
        ("MatchByName", "0"),
        ("IconUpdate", "1"),
        ("PatternFlags", "0"),
        ("Hidden", if mi.hidden { "1" } else { "0" }),
        ("MasterType", "2"),
    ];
    h.attrs = attrs.iter().map(|&(n, v)| (tx.intern(n), tx.intern(v))).collect();
    tx.set_master_header(mref, h).map_err(|e| sink.fail(line, "der Master", e))?;
    // the master page: its size and scale, the keywords
    let (len_unit, one) = if measure == Measure::Us { (Unit::Inches, 1.0) } else { (Unit::Millimeters, 1.0 / 25.4) };
    let ps = SheetRef::Master(mref);
    // a size from a formula: 0 until fit_sizes() writes the computed one
    let (w, h) = (mi.width.unwrap_or(0.0), mi.height.unwrap_or(0.0));
    // a scaled master (JD13, `scale:`): Visio's scale types 4 (metric) and 3 (custom)
    let scale_type = if mi.scale == 1.0 { 0.0 } else if measure == Measure::Us { 3.0 } else { 4.0 };
    for (name, v, u) in [
        ("PageWidth", w, Some(len_unit)),
        ("PageHeight", h, Some(len_unit)),
        ("PageScale", one, Some(len_unit)),
        ("DrawingScale", one * mi.scale, Some(len_unit)),
        ("DrawingSizeType", 1.0, None),
        ("DrawingScaleType", scale_type, None),
    ] {
        let a = single_addr(tx, name);
        sink.constant(tx, ps, &a, Value::Num(v), u, line)?;
    }
    if let Some(k) = &mi.keywords {
        let a = single_addr(tx, "ShapeKeywords");
        sink.text(tx, ps, &a, &k[lang], Some(Unit::Str), line)?;
    }
    // the master shape
    let layers: &[Layer] = mi.svg.as_ref().map_or(&[], |(a, _)| a.layers.as_slice());
    let kind = if layers.len() > 1 || !mi.parts.is_empty() { ShapeKind::Group } else { ShapeKind::Shape };
    let shape = tx.add_shape(Owner::Master(mref), None, kind).map_err(|e| sink.fail(line, "das Master-Shape", e))?;
    let refs = StyleRefs {
        line: Some(style_id(tx, file, &mi.styles[0], line)?),
        fill: Some(style_id(tx, file, &mi.styles[1], line)?),
        text: Some(style_id(tx, file, &mi.styles[2], line)?),
    };
    tx.set_shape_styles(shape, refs).map_err(|e| sink.fail(line, "die Formatvorlagen", e))?;
    let sheet = SheetRef::Shape(shape);
    // (JD26) the parts first, empty, so that every formula can name them (Sheet.2! ...)
    let mut subs: Vec<ShapeRef> = Vec::with_capacity(mi.parts.len());
    for p in &mi.parts {
        let sub = tx.add_shape(Owner::Master(mref), Some(shape), ShapeKind::Shape).map_err(|e| sink.fail(p.line, &format!("[Part {}]", p.name), e))?;
        subs.push(sub);
    }
    if !subs.is_empty() {
        sink.ids = core::iter::once(shape).chain(subs.iter().copied()).map(|r| tx.doc().shape(r).map_or(0, |x| x.id())).collect();
    }
    // the transform first, in Visio's order; a 1-D master's box follows its ends (Visio's own
    // formulas for a 1-D shape, README: 1D-Shapes) unless the master writes a cell itself
    const FIRST: &[&str] = &["PinX", "PinY", "Width", "Height", "LocPinX", "LocPinY"];
    const FIRST_1D: &[&str] = &["PinX", "PinY", "Width", "Height", "LocPinX", "LocPinY", "Angle", "BeginX", "BeginY", "EndX", "EndY"];
    let first = if mi.one_d { FIRST_1D } else { FIRST };
    for &name in first {
        let a = single_addr(tx, name);
        match mi.shape.iter().find(|c| c.name == name) {
            Some(c) => sink.put(tx, sheet, &a, c, Some(String::from(name)))?,
            None if mi.one_d => {
                let f = match name {
                    "PinX" => "GUARD((BeginX+EndX)/2)",
                    "PinY" => "GUARD((BeginY+EndY)/2)",
                    "Width" => "GUARD(SQRT((EndX-BeginX)^2+(EndY-BeginY)^2))",
                    "LocPinX" => "GUARD(Width*0.5)",
                    "LocPinY" => "GUARD(Height*0.5)",
                    "Angle" => "GUARD(ATAN2(EndY-BeginY,EndX-BeginX))",
                    _ => continue,
                };
                sink.formula(tx, sheet, &a, f, line)?;
            }
            None => match name {
                "PinX" => sink.constant(tx, sheet, &a, Value::Num(w / 2.0), Some(len_unit), line)?,
                "PinY" => sink.constant(tx, sheet, &a, Value::Num(h / 2.0), Some(len_unit), line)?,
                "LocPinX" => sink.formula(tx, sheet, &a, "Width*0.5", line)?,
                "LocPinY" => sink.formula(tx, sheet, &a, "Height*0.5", line)?,
                _ => {}
            },
        }
    }
    // (JD13) an SVG group's subshapes are its formats, no parts of their own: a click selects the
    // group only (SelectMode 0, as Visio names it), never one colour of it
    if kind == ShapeKind::Group && !mi.shape.iter().any(|c| c.name == "SelectMode") {
        let a = single_addr(tx, "SelectMode");
        sink.constant(tx, sheet, &a, Value::Num(0.0), None, line)?;
    }
    for c in mi.shape.iter().filter(|c| !first.contains(&c.name.as_str())) {
        let a = single_addr(tx, &c.name);
        sink.put(tx, sheet, &a, c, Some(c.name.clone()))?;
    }
    if let Some(t) = &mi.text {
        tx.set_text(shape, Some(Text::plain(&t[lang]))).map_err(|e| sink.fail(line, "der Text", e))?;
    }
    // the sections as written
    emit_sections(tx, &mut sink, sheet, &mi.sections, true)?;
    // (JD26) the parts: each a subshape the size of the group unless it says otherwise (GUARD: Visio
    // does not scale it when the group is resized, the formulas do), its
    // cells, sections and text; the text's five languages in its own User.JFD_Text (the app
    // shows the current language's on dropping, instance::localize_parts)
    for (p, &sub) in mi.parts.iter().zip(&subs) {
        let prefs = StyleRefs {
            line: Some(style_id(tx, file, &p.styles[0], p.line)?),
            fill: Some(style_id(tx, file, &p.styles[1], p.line)?),
            text: Some(style_id(tx, file, &p.styles[2], p.line)?),
        };
        tx.set_shape_styles(sub, prefs).map_err(|e| sink.fail(p.line, "die Formatvorlagen", e))?;
        let ss = SheetRef::Shape(sub);
        for &name in FIRST {
            let a = single_addr(tx, name);
            match p.shape.iter().find(|c| c.name == name) {
                Some(c) => sink.put(tx, ss, &a, c, None)?,
                None => {
                    let f = match name {
                        "PinX" => "GUARD(Sheet.1!Width*0.5)",
                        "PinY" => "GUARD(Sheet.1!Height*0.5)",
                        "Width" => "GUARD(Sheet.1!Width)",
                        "Height" => "GUARD(Sheet.1!Height)",
                        "LocPinX" => "GUARD(Width*0.5)",
                        _ => "GUARD(Height*0.5)",
                    };
                    sink.formula(tx, ss, &a, f, p.line)?;
                }
            }
        }
        for c in p.shape.iter().filter(|c| !FIRST.contains(&c.name.as_str())) {
            let a = single_addr(tx, &c.name);
            sink.put(tx, ss, &a, c, None)?;
        }
        emit_sections(tx, &mut sink, ss, &p.sections, false)?;
        if let Some(t) = &p.text {
            tx.set_text(sub, Some(Text::plain(&t[lang]))).map_err(|e| sink.fail(p.line, &format!("der Text von [Part {}]", p.name), e))?;
            let v = tx.intern(&join_fields(&[&t[0], &t[1], &t[2], &t[3], &t[4]]));
            tx.set_user(ss, instance::JFD_TEXT, Value::Text(v), Some(Unit::Str)).map_err(|e| sink.fail(p.line, instance::JFD_TEXT, e))?;
        }
    }
    // the SVG geometry: on the master shape, or one subshape per format
    if let Some((art, svg_line)) = &mi.svg {
        // line weights are on paper: a scaled master's drawing units shrink by its scale
        let lw = formula::math::sqrt((w / art.vb[2]) * (h / art.vb[3])) / mi.scale;
        if layers.len() == 1 {
            let own: Vec<&str> = mi.shape.iter().map(|c| c.name.as_str()).collect();
            emit_layer(tx, &mut sink, sheet, &layers[0], lw, &own, *svg_line)?;
        } else {
            let gid = tx.doc().shape(shape).map_or(0, |s| s.id());
            for layer in layers {
                let sub = tx.add_shape(Owner::Master(mref), Some(shape), ShapeKind::Shape).map_err(|e| sink.fail(*svg_line, "ein Untershape", e))?;
                tx.set_shape_styles(sub, refs).map_err(|e| sink.fail(*svg_line, "die Formatvorlagen", e))?;
                let ss = SheetRef::Shape(sub);
                for (name, f) in [
                    ("PinX", format!("Sheet.{gid}!Width*0.5")),
                    ("PinY", format!("Sheet.{gid}!Height*0.5")),
                    ("Width", format!("Sheet.{gid}!Width")),
                    ("Height", format!("Sheet.{gid}!Height")),
                    ("LocPinX", String::from("Width*0.5")),
                    ("LocPinY", String::from("Height*0.5")),
                ] {
                    let a = single_addr(tx, name);
                    sink.formula(tx, ss, &a, &f, *svg_line)?;
                }
                emit_layer(tx, &mut sink, ss, layer, lw, &[], *svg_line)?;
            }
        }
    }
    // the JFD_ cells of the master page (README, diagram_core::instance)
    let j5 = |t: &[String; 5]| join_fields(&[&t[0], &t[1], &t[2], &t[3], &t[4]]);
    let mut user: Vec<(String, String)> = alloc::vec![(String::from(instance::JFD_NAME), j5(&mi.names)), (String::from(instance::JFD_PROMPT), j5(&mi.prompts))];
    if let Some(k) = &mi.keywords {
        user.push((String::from(instance::JFD_KEYWORDS), j5(k)));
    }
    if let Some(t) = &mi.text {
        user.push((String::from(instance::JFD_TEXT), j5(t)));
    }
    for (name, value) in user {
        let v = tx.intern(&value);
        tx.set_user(ps, &name, Value::Text(v), Some(Unit::Str)).map_err(|e| sink.fail(line, &name, e))?;
    }
    if quick {
        tx.set_user(ps, instance::JFD_QUICK, Value::Num(1.0), None).map_err(|e| sink.fail(line, instance::JFD_QUICK, e))?;
    }
    for (i, (r, t)) in sink.tr.iter().enumerate() {
        let name = format!("{}{}", instance::JFD_TR, i + 1);
        let v = tx.intern(&join_fields(&[r, &t[0], &t[1], &t[2], &t[3], &t[4]]));
        tx.set_user(ps, &name, Value::Text(v), Some(Unit::Str)).map_err(|e| err(file, line, format!("{name} lässt sich nicht schreiben ({e:?})")))?;
    }
    Ok(mref)
}

// the sections of the master shape or of a part as written; `tr`: language-suffixed cells
// are translated (JFD_Tr, the master shape only)
fn emit_sections(tx: &mut Transaction, sink: &mut Sink, sheet: SheetRef, sections: &[Sec0], tr: bool) -> Result<(), SourceError> {
    for sec in sections {
        match sec.kind {
            SecKind::Shape => {}
            SecKind::Geometry => {
                let sk = tx.add_section(sheet, "Geometry").map_err(|e| sink.fail(sec.line, "[Geometry]", e))?;
                for c in &sec.cells {
                    let a = CellAddr::section(sk, tx.intern(&c.name));
                    sink.put(tx, sheet, &a, c, None)?;
                }
                for row in &sec.rows {
                    let rk = tx.add_row(sheet, sk, None, Some(&row.head)).map_err(|e| sink.fail(row.line, &row.head, e))?;
                    for c in &row.cells {
                        let a = CellAddr::row(sk, rk, tx.intern(&c.name));
                        sink.put(tx, sheet, &a, c, None)?;
                    }
                }
            }
            SecKind::Indexed | SecKind::Named => {
                let sk = SecKey::one(tx.intern(sec.model));
                for (n, row) in sec.rows.iter().enumerate() {
                    let (rk, rname) = if sec.kind == SecKind::Named { (RowKey::Named(tx.intern(&row.head)), row.head.clone()) } else { (RowKey::Index(n as u32), n.to_string()) };
                    if !tx.doc().has_row(sheet, sk, rk) {
                        tx.add_row(sheet, sk, Some(rk), None).map_err(|e| sink.fail(row.line, &row.head, e))?;
                    }
                    for c in &row.cells {
                        let a = CellAddr::row(sk, rk, tx.intern(&c.name));
                        sink.put(tx, sheet, &a, c, tr.then(|| format!("{}.{rname}.{}", sec.model, c.name)))?;
                    }
                }
            }
        }
    }
    Ok(())
}

// one SVG format: its line and fill cells (unless the master sets them itself) and its sections
fn emit_layer(tx: &mut Transaction, sink: &mut Sink, sheet: SheetRef, layer: &Layer, lw: f64, own: &[&str], line: u32) -> Result<(), SourceError> {
    let f = &layer.fmt;
    let mut cells: Vec<(&str, Value0)> = Vec::new();
    if let Some(c) = &f.fill {
        cells.push(("FillForegnd", Value0::Color(c.clone())));
        cells.push(("FillBkgnd", Value0::Color(c.clone())));
        cells.push(("FillPattern", Value0::Num(1.0, None)));
        if f.fill_alpha < 1.0 {
            cells.push(("FillForegndTrans", Value0::Num(1.0 - f.fill_alpha, Some(Unit::Percent))));
            cells.push(("FillBkgndTrans", Value0::Num(1.0 - f.fill_alpha, Some(Unit::Percent))));
        }
    }
    if let Some(c) = &f.stroke {
        cells.push(("LineColor", Value0::Color(c.clone())));
        cells.push(("LinePattern", Value0::Num(1.0, None)));
        cells.push(("LineWeight", Value0::Num(f.width * lw, Some(Unit::Points))));
        if f.stroke_alpha < 1.0 {
            cells.push(("LineColorTrans", Value0::Num(1.0 - f.stroke_alpha, Some(Unit::Percent))));
        }
        if let Some(cap) = f.cap {
            cells.push(("LineCap", Value0::Num(cap as f64, None)));
        }
    }
    for (name, v) in cells {
        if own.contains(&name) {
            continue;
        }
        let a = single_addr(tx, name);
        sink.put(tx, sheet, &a, &Cell0 { name: String::from(name), line, val: CellVal::Plain(v), file: None }, None)?;
    }
    let xy = |p: [f64; 2]| (format!("Width*{}", frac(p[0])), format!("Height*{}", frac(p[1])));
    // (JD13) consecutive sub-paths with the same NoFill / NoLine share one Geometry section, each
    // starting with its MoveTo (Visio fills a shape's figures together by the even-odd rule
    // either way); hundreds of sections in one shape made building and drawing slow
    let mut open: Option<(SecKey, bool, bool)> = None;
    for sec in &layer.sections {
        let sk = match open {
            Some((sk, f, l)) if f == sec.no_fill && l == sec.no_line => sk,
            _ => {
                let sk = tx.add_section(sheet, "Geometry").map_err(|e| sink.fail(line, "[Geometry]", e))?;
                for (flag, name) in [(sec.no_fill, "NoFill"), (sec.no_line, "NoLine")] {
                    if flag {
                        let a = CellAddr::section(sk, tx.intern(name));
                        sink.constant(tx, sheet, &a, Value::Num(1.0), Some(Unit::Bool), line)?;
                    }
                }
                open = Some((sk, sec.no_fill, sec.no_line));
                sk
            }
        };
        let (x, y) = xy(sec.start);
        geo_row(tx, sink, (sheet, sk), "MoveTo", &[("X", Value0::Formula(x)), ("Y", Value0::Formula(y))], line)?;
        for s in &sec.segs {
            match *s {
                Seg::Line(p) => {
                    let (x, y) = xy(p);
                    geo_row(tx, sink, (sheet, sk), "LineTo", &[("X", Value0::Formula(x)), ("Y", Value0::Formula(y))], line)?;
                }
                Seg::Cubic(c1, c2, p) => {
                    // one cubic Bézier piece: degree 3, knots 0 0 0 0 1 1 1 1, all weights 1,
                    // the control points as fractions of Width and Height (types 0)
                    let (x, y) = xy(p);
                    let e = format!("NURBS(1,3,0,0,{},{},0,1,{},{},0,1)", frac(c1[0]), frac(c1[1]), frac(c2[0]), frac(c2[1]));
                    let cells = [
                        ("X", Value0::Formula(x)),
                        ("Y", Value0::Formula(y)),
                        ("A", Value0::Num(0.0, None)),
                        ("B", Value0::Num(1.0, None)),
                        ("C", Value0::Num(0.0, None)),
                        ("D", Value0::Num(1.0, None)),
                    ];
                    let rk = geo_row(tx, sink, (sheet, sk), "NURBSTo", &cells, line)?;
                    // E with U="NURBS" (the unit stays when the formula is set)
                    let a = CellAddr::row(sk, rk, tx.intern("E"));
                    sink.text(tx, sheet, &a, &e, Some(Unit::Nurbs), line)?;
                    sink.formula(tx, sheet, &a, &e, line)?;
                }
            }
        }
    }
    Ok(())
}

// a Geometry row of its own with formula and number cells -> its key
fn geo_row(tx: &mut Transaction, sink: &mut Sink, at: (SheetRef, SecKey), kind: &str, cells: &[(&str, Value0)], line: u32) -> Result<RowKey, SourceError> {
    let (sheet, sk) = at;
    let rk = tx.add_row(sheet, sk, None, Some(kind)).map_err(|e| sink.fail(line, kind, e))?;
    for (name, v) in cells {
        let a = CellAddr::row(sk, rk, tx.intern(name));
        match v {
            Value0::Formula(t) => sink.formula(tx, sheet, &a, t, line)?,
            Value0::Num(x, u) => sink.constant(tx, sheet, &a, Value::Num(*x), *u, line)?,
            _ => {}
        }
    }
    Ok(rk)
}

// ---------------------------------------------------------------- checks after writing

// the master a sheet belongs to
fn master_of(s: SheetRef) -> Option<MasterRef> {
    match s {
        SheetRef::Master(m) => Some(m),
        SheetRef::Shape(r) => match r.owner {
            Owner::Master(m) => Some(m),
            Owner::Page(_) => None,
        },
        _ => None,
    }
}

// a cell part as written in formulas
fn show_part(p: &CellPart) -> String {
    match p {
        CellPart::None => String::new(),
        CellPart::Single(c) => c.to_string(),
        CellPart::Section { section, index, cell } => format!("{section}{}.{cell}", index + 1),
        CellPart::Indexed { section, index, row, cell } if *section == "Geometry" => format!("Geometry{}.{cell}{row}", index + 1),
        CellPart::Indexed { section, row, cell, .. } => format!("{section}.{cell}[{}]", row + 1),
        CellPart::Named { section, row, cell } => format!("{section}.{row}.{cell}"),
        CellPart::Unknown(t) => t.to_string(),
    }
}

// every reference of a formula names something that is there
fn check_refs(doc: &Document, f: &FormulaAt) -> Result<(), String> {
    let p = formula::parse(&f.text).map_err(|e| format!("Formel nicht lesbar ab Zeichen {}", e.pos + 1))?;
    let owner = match f.sheet {
        SheetRef::Shape(r) => Some(r.owner),
        SheetRef::Master(m) => Some(Owner::Master(m)),
        _ => None,
    };
    for r in p.refs() {
        let sheet = match &r.sheet {
            SheetName::Local => Some(f.sheet),
            SheetName::TheDoc => Some(SheetRef::Document),
            SheetName::ThePage => master_of(f.sheet).map(SheetRef::Master),
            SheetName::Id(n) => owner.and_then(|o| doc.shape_by_id(o, *n)).map(SheetRef::Shape),
            SheetName::Named(name) => owner.and_then(|o| doc.all_shapes(o).find(|&s| doc.shape(s).is_some_and(|x| x.name_u() == **name))).map(SheetRef::Shape),
            SheetName::Master(name, sub) => doc.master_by_name_u(name).and_then(|m| match sub {
                Some(id) => doc.shape_by_id(Owner::Master(m), *id).map(SheetRef::Shape),
                None => Some(SheetRef::Master(m)),
            }),
            SheetName::Style(name) => doc.style_by_name_u(name).map(SheetRef::Style),
            SheetName::Page(..) => None,
            // a sheet a function gives (CONTAINERSHEETREF(..)!): known only on a page
            SheetName::Call(_) => continue,
        };
        let Some(sheet) = sheet else {
            return Err(String::from("Bezug auf ein Blatt, das es nicht gibt"));
        };
        let addr = match &r.cell {
            CellPart::None => continue,
            CellPart::Unknown(t) => return Err(format!("unbekannter Abschnitt im Bezug {t}")),
            CellPart::Single(name) => match doc.sym(name) {
                Some(s) => Some(CellAddr::single(s)),
                None if single_cell_known(name) => continue,
                None => None,
            },
            CellPart::Section { section, index, cell } => doc.sym(section).zip(doc.sym(cell)).map(|(s, c)| CellAddr::section(SecKey { name: s, index: *index }, c)),
            CellPart::Indexed { section, index, row, cell } => {
                doc.sym(section).zip(doc.sym(cell)).map(|(s, c)| CellAddr::row(SecKey { name: s, index: *index }, RowKey::Index(*row), c))
            }
            CellPart::Named { section, row, cell } => match (doc.sym(section), doc.sym(row), doc.sym(cell)) {
                (Some(s), Some(r), Some(c)) => Some(CellAddr::row(SecKey::one(s), RowKey::Named(r), c)),
                _ => None,
            },
        };
        let there = addr.is_some_and(|a| doc.resolve(sheet, &a).exists())
            || matches!(&r.cell, CellPart::Single(name) if single_cell_known(name));
        if !there {
            return Err(format!("Bezug ins Leere: {}", show_part(&r.cell)));
        }
    }
    Ok(())
}

fn keep_message(k: &Keep) -> String {
    match k {
        Keep::Parse(e) => format!("Formel nicht lesbar ab Zeichen {}", e.pos + 1),
        Keep::Unknown(n) => format!("unbekannte Funktion {n}"),
        Keep::NotComputed(n) => format!("{n} lässt sich beim Bauen nicht berechnen"),
        Keep::Reference => String::from("Bezug auf einen unbekannten Abschnitt"),
        Keep::Deep => String::from("Formel zu tief verschachtelt"),
    }
}

// the formula engine computes every formula; a kept cell or an error value is a build error
fn compute(doc: &mut Document, infos: &[MasterInfo], mrefs: &[MasterRef], formulas: &[FormulaAt]) -> Result<(), SourceError> {
    let res = Engine::new().calculate(doc);
    let mut worst: Option<(usize, u32, String, Option<String>)> = None;
    let mut note = |s: SheetRef, a: &CellAddr, msg: String| {
        let found = formulas.iter().find(|f| f.sheet == s && f.addr == *a).map(|f| (f.master, f.line, format!("{msg} in „{}“", f.text), f.file.clone()));
        let at = found.or_else(|| {
            let m = master_of(s)?;
            let ix = mrefs.iter().position(|&x| x == m)?;
            Some((ix, infos[ix].line, format!("{msg} ({})", doc.str(a.cell)), None))
        });
        if let Some(at) = at {
            if worst.as_ref().is_none_or(|w| (at.0, at.1) < (w.0, w.1)) {
                worst = Some(at);
            }
        }
    };
    for (s, a, o) in &res.cells {
        match o {
            Outcome::Kept(k) => note(*s, a, keep_message(k)),
            Outcome::Value(v) => {
                if let Some(e) = v.err() {
                    note(*s, a, format!("Formel ergibt {}", e.as_str()));
                }
            }
        }
    }
    for (s, a) in &res.cycles {
        note(*s, a, String::from("Zirkelbezug"));
    }
    if let Some((m, line, msg, file)) = worst {
        return Err(err(file.as_deref().unwrap_or(&infos[m].file), line, msg));
    }
    let mut tx = doc.transaction("Formeln");
    res.write(&mut tx).map_err(|e| err("stencil.txt", 0, format!("Formelergebnisse lassen sich nicht schreiben ({e:?})")))?;
    let _ = tx.commit();
    Ok(())
}

// a 2-D master whose Width or Height is a formula: the master page gets the computed size and
// the master shape the pin in its middle (constants, as for the other masters)
fn fit_sizes(doc: &mut Document, infos: &[MasterInfo], mrefs: &[MasterRef], measure: Measure) -> Result<(), SourceError> {
    let unit = if measure == Measure::Us { Unit::Inches } else { Unit::Millimeters };
    let mut tx = doc.transaction("Größen");
    for (mi, &m) in infos.iter().zip(mrefs) {
        if mi.width.is_some() && mi.height.is_some() {
            continue;
        }
        let Some(slot) = tx.doc().master(m).and_then(|x| x.master_shape()) else { continue };
        let sheet = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
        let num = |doc: &Document, n: &str| doc.sym(n).and_then(|s| doc.resolve(sheet, &CellAddr::single(s)).num()).unwrap_or(0.0);
        let (w, h) = (num(tx.doc(), "Width"), num(tx.doc(), "Height"));
        if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
            return Err(err(&mi.file, mi.line, format!("Width und Height ergeben beim Bauen keine positive Größe ({w} x {h} in)")));
        }
        for (s, name, v) in [(SheetRef::Master(m), "PageWidth", w), (SheetRef::Master(m), "PageHeight", h), (sheet, "PinX", w * 0.5), (sheet, "PinY", h * 0.5)] {
            let a = single_addr(&mut tx, name);
            tx.set_constant(s, &a, Value::Num(v), Some(unit)).map_err(|e| err(&mi.file, mi.line, format!("{name} lässt sich nicht schreiben ({e:?})")))?;
        }
    }
    let _ = tx.commit();
    Ok(())
}

// ---------------------------------------------------------------- the API

/// build the stencil of one folder for one measure (see the module comment)
pub fn build(files: &dyn Fn(&str) -> Option<String>, measure: Measure) -> Result<Built, SourceError> {
    let text = files("stencil.txt").ok_or_else(|| err("stencil.txt", 0, "stencil.txt fehlt"))?;
    let st = stencil_info(&text, measure)?;
    let mut infos: Vec<MasterInfo> = Vec::with_capacity(st.masters.len());
    for stem in &st.masters {
        let file = format!("{stem}.master");
        let t = files(&file).ok_or_else(|| err("stencil.txt", st.masters_line, format!("die Master-Datei {file} fehlt")))?;
        let mi = master_info(&file, &t, measure, files)?;
        if let Some(prev) = infos.iter().find(|p| p.name_u == mi.name_u) {
            return Err(err(&file, mi.line, format!("NameU „{}“ hat schon {}", mi.name_u, prev.file)));
        }
        infos.push(mi);
    }
    if infos.is_empty() {
        return Err(err("stencil.txt", st.masters_line, "masters: keine Master"));
    }
    let mut doc = vsdx::new_document(DocKind::Stencil, measure);
    let mut formulas: Vec<FormulaAt> = Vec::new();
    let mut mrefs: Vec<MasterRef> = Vec::with_capacity(infos.len());
    {
        let mut tx = doc.transaction("Schablone bauen");
        let names = join_fields(&[&st.names[0], &st.names[1], &st.names[2], &st.names[3], &st.names[4]]);
        for (name, value) in [(instance::JFD_NAME, names.as_str()), (instance::JFD_CATEGORY, st.category.as_str())] {
            let v = tx.intern(value);
            tx.set_user(SheetRef::Document, name, Value::Text(v), Some(Unit::Str))
                .map_err(|e| err("stencil.txt", 1, format!("{name} lässt sich nicht schreiben ({e:?})")))?;
        }
        for (i, mi) in infos.iter().enumerate() {
            mrefs.push(emit_master(&mut tx, &st, mi, i, measure, &mut formulas)?);
        }
        let _ = tx.commit();
    }
    for f in &formulas {
        check_refs(&doc, f).map_err(|m| err(f.file.as_deref().unwrap_or(&infos[f.master].file), f.line, format!("{m} in „{}“", f.text)))?;
    }
    compute(&mut doc, &infos, &mrefs, &formulas)?;
    fit_sizes(&mut doc, &infos, &mrefs, measure)?;
    for (mi, &m) in infos.iter().zip(&mrefs) {
        let ico = stencil_icon::master_icon(&mut doc, m).map_err(|e| err(&mi.file, mi.line, format!("das Symbol lässt sich nicht rendern ({e:?})")))?;
        let sym = doc.intern(&instance::base64(&ico));
        if let Some(master) = doc.master_mut(m) {
            master.icon = Some(sym);
        }
    }
    Ok(Built { doc, stencil: st.stencil, file: st.file, category: st.category, measure })
}

/// build the stencil and write it as .vssx bytes (vsdx::write with the default options)
pub fn build_vssx(files: &dyn Fn(&str) -> Option<String>, measure: Measure) -> Result<(Built, Vec<u8>), SourceError> {
    let built = build(files, measure)?;
    let bytes = vsdx::write(&built.doc, &WriteOptions::DEFAULT).map_err(|e| err("stencil.txt", 0, format!("die .vssx lässt sich nicht schreiben: {e}")))?;
    Ok((built, bytes))
}
