// template -- the template build tool (JD13): the text sources of a template into template
// documents and .vstx files; and what the app does with a template, built-in or the user's
//
//   template::build(&files, Measure::Metric, &stencils) -> Result<Vec<BuiltTemplate>, SourceError>
//   template::build_vstx(&files, Measure::Us, &stencils) -> Result<Vec<(BuiltTemplate, Vec<u8>)>, ..>
//   BuiltTemplate { doc, template, file, starter, category, order, measure, listed }
//                                   every form of one template: the blank one first, then the
//                                   starter diagrams in `starters` order (file "flowchart",
//                                   "flowchart-sample"); listed: false for a blank form the
//                                   catalogue does not show (`blank: false`)
//   built.vstx_name()               "flowchart_m.vstx", "cff-vertical_u.vstx"
//
//   stencil_windows(&doc) -> Vec<StencilWindow { document, active }>   the stencils a template
//                                   opens: the Stencil windows of the windows part (Visio's or
//                                   ours) by StencilGroupPos, else in document order; active =
//                                   WindowState bit 0x4000000 (visWSActive: unfolded)
//   set_stencil_windows(&mut doc, &list)   the windows part for these stencils (EX-11)
//   info(&doc) -> Info              the catalogue's cells (the document sheet's JFD_ cells)
//   registers(&doc)                 the context registers a template brought (VL-04)
//   to_drawing(&mut doc, lang)      a new drawing from a template (DF-05, VL-03)
//
// `files(name)` gives the text of "template.txt", of "<name>.starter" and of the rules file, all
// of one template folder (None: there is none); `stencils(stem)` the built stencil of the same
// measure whose output stem is `stem` (stencil.txt `file`; None: there is none, a build error).
// The source format and what goes into the file: apps/JFDiagrammer/templates/README.md.  The
// app's build_steps.rs calls this for each template folder and both measures after the
// stencils.  The parser is template_src.rs (the stencils' common rules, stencil_src.rs).
//
// How a template is built, in this order:
// 1. template.txt, the starter files and the rules file are parsed and checked for the measure:
//    header keys, sections, row heads, cell names, the five languages, constants where the
//    format wants them (templates hold no formulas but the rules' Filter and Test, which must
//    parse), the stencils named exist, the starters' shapes name masters and rows that are
//    there.  A ".metric" / ".us" line replaces the line without suffix.
// 2. A new template (vsdx::new_document: the own styles and the design NetLab) gets the cells
//    of [Style ...], the first page's name and the cells of [PageSheet] (PrintPageOrientation
//    follows the size when a size is given without it), the pages of [Pages] (their page sheet
//    a copy of the first page's), background pages, the layers of [Layer] on every foreground
//    page, the design and its variant on every page (design::apply::apply), the JFD_ cells of
//    the document sheet and the page sheets, the windows part with the stencils
//    (set_stencil_windows) and the rule set's part.
// 3. Each form is a copy of that: the blank form, then per starter diagram its JFD_Starter (and
//    own description) and the shapes of [Shapes] on the first page in file order -- master
//    instances as JFDiagrammer drops them (instance::import_master, instance::place in the
//    build language), dynamic connectors glued to the whole shapes at both ends
//    (glue::new_connector), cross-functional flowcharts (structure::cff::create).  JFD_Default
//    marks the form the catalogue chooses first.
// 4. The formula engine computes every formula of each form and the values are stored: no
//    stale value goes into the file.
// The same sources always give the same documents and the same bytes.
//
// The windows part (visio/windows.xml), as Visio writes it: a drawing window (WindowType
// 'Drawing') and per stencil a Window with WindowType 'Stencil', Document (the file name of a
// built-in stencil, "jfd_standard_m.vssx" (stencil::file_name); a path for others), ParentWindow (the drawing window),
// WindowState 1025 (visWSDockedLeft | visWSAnchorMerged) or 67109889 (| visWSActive: the
// unfolded one) and the children StencilGroup 10 and StencilGroupPos 1, 2 ... in the Shapes
// panel's order.  set_stencil_windows keeps the part's other windows (the drawing window,
// Visio's panes) as written and replaces its stencil windows; without a drawing window it
// writes a minimal one of its own (maximized, on the first page).
//
// The rule set's part visio/validation.xml (relationship …/visio/2010/relationships/validation
// from the main part, which pack.rs adds for a part of Visio's own kind): Validation >
// ValidationProperties, RuleSets > RuleSet (ID, NameU, Name, Description, RuleSetFlags) > Rule
// (ID, NameU, Category, Description, RuleTarget, RuleFilter, RuleTest), texts in the build
// language; all five in the document sheet's JFD_RuleSetN ("NameU|5 names|5 descriptions") and
// JFD_RuleN ("RuleSet.Rule|5 descriptions|5 categories").
//
// to_drawing: DocKind::Drawing; page names (the page sheet's JFD_Name), layer names
// (JFD_LayerN, row IX N-1), shape texts (the shape's JFD_Text, else its master's text where the
// shape shows one of the master's texts), instance names and translated cells (the master's
// JFD_Name and JFD_TrN where the instance shows one of them, as instance::place sets them) and
// the rule set's names, descriptions and categories in the chosen language; then the cells of
// the catalogue go (JFD_Name, JFD_Description, JFD_Category, JFD_Order, JFD_Starter,
// JFD_Default, JFD_Text, JFD_LayerN), JFD_Template, JFD_Registers, JFD_Tip and the rules'
// translations stay.  The sheets changed are recalculated, nothing else.
//
// The sources are files of the repository, trusted but checked all the same; a template read
// from a file (stencil_windows, info, to_drawing) is untrusted: no panic on any input, counts
// bounded, no recursion.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::design::{self, apply::Scope};
use diagram_core::formula::{self, Engine};
use diagram_core::glue::{self, Target};
use diagram_core::instance::{self, join_fields, split_fields, MasterTexts, LANGS};
use diagram_core::structure::cff;
use diagram_core::{
    CellAddr, Dimension, DocKind, Document, EditError, Owner, PageRef, Part, RowKey, SecKey, ShapeRef, SheetRef, StyleRefs, Text,
    Transaction, Unit, Value,
};

use crate::stencil::{self, SourceError, CATEGORIES};
use crate::stencil_src::{self, err, Entry, Value0};
use crate::template_src::{self, SecSpec, TKind, TSource};
use crate::vsdx::{self, Measure, WriteOptions, CT_WINDOWS};
use crate::write::{escape, DECL};
use crate::xml::{self, Event, Reader, NS_REL, NS_VISIO};

/// one form of a template, built
#[derive(Clone, Debug)]
pub struct BuiltTemplate {
    /// the template (DocKind::Template)
    pub doc: Document,
    /// the template's universal name (template.txt `template`)
    pub template: String,
    /// the output stem: `file`, or `file-starter` for a starter diagram
    pub file: String,
    /// the starter diagram's universal name (None: the blank form)
    pub starter: Option<String>,
    /// the catalogue's category (template.txt `category`)
    pub category: String,
    /// the place in the catalogue, smaller first
    pub order: u32,
    /// the measure it was built for
    pub measure: Measure,
    /// whether the catalogue shows this form (false: the blank form of `blank: false`)
    pub listed: bool,
}

impl BuiltTemplate {
    /// the file name of the .vstx: `<file>_m.vstx` (metric) or `<file>_u.vstx` (US)
    pub fn vstx_name(&self) -> String {
        format!("{}_{}.vstx", self.file, if self.measure == Measure::Us { "u" } else { "m" })
    }
}

/// a stencil a template opens: its Document as written (a built-in stencil's file name, else a
/// path) and whether it is the one unfolded
#[derive(Clone, Debug, PartialEq)]
pub struct StencilWindow {
    pub document: String,
    pub active: bool,
}

/// what the catalogue reads from a template (the document sheet's JFD_ cells; empty or None
/// where a cell is missing, as in templates that were not built here)
#[derive(Clone, Debug, Default)]
pub struct Info {
    /// the universal name (JFD_Template)
    pub template: String,
    /// the title in five languages (JFD_Name)
    pub name: Option<[String; 5]>,
    /// the description (JFD_Description)
    pub description: Option<[String; 5]>,
    /// the category (JFD_Category)
    pub category: String,
    /// the place in the catalogue (JFD_Order; 0 without)
    pub order: u32,
    /// the starter diagram's name (JFD_Starter; None: the blank form)
    pub starter: Option<[String; 5]>,
    /// the form the catalogue chooses first (JFD_Default)
    pub default: bool,
    /// the context registers (JFD_Registers)
    pub registers: Vec<String>,
}

/// the universal name of the template (document sheet; stays in drawings)
pub const JFD_TEMPLATE: &str = "JFD_Template";
/// the description in the catalogue (document sheet)
pub const JFD_DESCRIPTION: &str = "JFD_Description";
/// the place in the catalogue (document sheet)
pub const JFD_ORDER: &str = "JFD_Order";
/// the starter diagram's name (document sheet of a starter form)
pub const JFD_STARTER: &str = "JFD_Starter";
/// 1 in the form the catalogue chooses first (document sheet)
pub const JFD_DEFAULT: &str = "JFD_Default";
/// the context registers, ';'-separated (document sheet; stays in drawings)
pub const JFD_REGISTERS: &str = "JFD_Registers";
/// TRUE on the tip shape of a starter diagram
pub const JFD_TIP: &str = "JFD_Tip";
/// JFD_RuleSet1 ...: "NameU|5 names|5 descriptions" (document sheet)
pub const JFD_RULE_SET: &str = "JFD_RuleSet";
/// JFD_Rule1 ...: "RuleSet.Rule|5 descriptions|5 categories" (document sheet)
pub const JFD_RULE: &str = "JFD_Rule";
/// JFD_Layer1 ...: a layer's name in five languages (page sheet; layer IX N-1)
pub const JFD_LAYER: &str = "JFD_Layer";

/// the context registers a template may bring (VL-04)
pub const REGISTERS: [&str; 1] = ["cff"];

const CT_VALIDATION: &str = "application/vnd.ms-visio.validation+xml";
const REL_VALIDATION: &str = "http://schemas.microsoft.com/visio/2010/relationships/validation";
const REL_WINDOWS: &str = "http://schemas.microsoft.com/visio/2010/relationships/windows";

// WindowState of a stencil window: docked left, merged; | visWSActive for the unfolded one
const WS_DOCKED: u32 = 1025;
const WS_ACTIVE: u32 = 0x400_0000;
// WindowState of the drawing window of our own: maximized
const WS_MAXIMIZED: u32 = 0x4000_0000;

const MAX_LIST: usize = 1_000;
const MAX_SHAPES: usize = 10_000;
const MAX_LANES: usize = 100;

const F: &str = "template.txt";

/// the sections of each file kind
const TEMPLATE_SECS: &[SecSpec] = &[("PageSheet", TKind::Cells, false), ("Layer", TKind::Indexed, false), ("Style", TKind::Cells, true), ("Pages", TKind::Named, false)];
const STARTER_SECS: &[SecSpec] = &[("Shapes", TKind::Named, false)];
const RULES_SECS: &[SecSpec] = &[("Rules", TKind::Named, false)];

/// the page sheet's cells [PageSheet] may set
const PAGE_CELLS: &[&str] = &[
    // size and scale
    "PageWidth", "PageHeight", "PageScale", "DrawingScale", "DrawingSizeType", "DrawingScaleType", "DrawingResizeType",
    "InhibitSnap", "PageLockReplace", "PageLockDuplicate", "UIVisibility", "PageShapeSplit",
    // shadows
    "ShdwOffsetX", "ShdwOffsetY", "ShdwType", "ShdwObliqueAngle", "ShdwScaleFactor",
    // print setup
    "PrintPageOrientation", "PaperKind", "PaperSource", "PageLeftMargin", "PageRightMargin", "PageTopMargin", "PageBottomMargin",
    "ScaleX", "ScaleY", "PagesX", "PagesY", "CenterX", "CenterY", "OnPage", "PrintGrid",
    // rulers and grid
    "XRulerDensity", "YRulerDensity", "XRulerOrigin", "YRulerOrigin", "XGridDensity", "YGridDensity", "XGridSpacing",
    "YGridSpacing", "XGridOrigin", "YGridOrigin",
    // layout and routing
    "PlaceStyle", "PlaceDepth", "PlowCode", "ResizePage", "DynamicsOff", "EnableGrid", "CtrlAsInput", "LineAdjustFrom",
    "LineAdjustTo", "BlockSizeX", "BlockSizeY", "AvenueSizeX", "AvenueSizeY", "RouteStyle", "PageLineJumpDirX",
    "PageLineJumpDirY", "LineToNodeX", "LineToNodeY", "LineToLineX", "LineToLineY", "LineJumpFactorX", "LineJumpFactorY",
    "LineJumpCode", "LineJumpStyle", "PlaceFlip", "LineRouteExt",
];

/// the cells of a layer row besides Name
const LAYER_FLAGS: [&str; 6] = ["Visible", "Print", "Active", "Lock", "Snap", "Glue"];

/// the first page's default names
const PAGE1: [&str; 5] = ["Seite-1", "Page-1", "Página-1", "Pagina-1", "Page-1"];

// ---------------------------------------------------------------- resolved cells

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
}

fn join5(t: &[String; 5]) -> String {
    join_fields(&[&t[0], &t[1], &t[2], &t[3], &t[4]])
}

// five values of a '|'-joined text (missing ones empty)
fn five_of(text: &str) -> [String; 5] {
    let mut v = split_fields(text).into_iter();
    core::array::from_fn(|_| v.next().unwrap_or_default())
}

// the cells of one scope for measure m: `allowed` names, `langs` the names that are texts in
// five languages; templates hold constants only
fn cells_of(file: &str, entries: &[Entry], m: Measure, allowed: &dyn Fn(&str) -> bool, place: &str, langs: &[&str]) -> Result<Vec<Cell0>, SourceError> {
    let resolved = stencil_src::resolve(file, entries, m)?;
    let mut out: Vec<(Cell0, u8)> = Vec::new();
    for e in resolved {
        let text = langs.contains(&e.name.as_str());
        if !text && !allowed(&e.name) {
            return Err(err(file, e.line, format!("unbekannte Zelle {} in {place}", e.name)));
        }
        match (e.lang, text) {
            (Some(l), true) => match out.iter_mut().find(|(c, _)| c.name == e.name) {
                Some((Cell0 { val: CellVal::Text(t), .. }, have)) => {
                    t[l] = stencil_src::unquote(&e.value);
                    *have |= 1 << l;
                }
                Some(_) => return Err(err(file, e.line, format!("{}: mit und ohne Sprachendung", e.name))),
                None => {
                    let mut t: [String; 5] = Default::default();
                    t[l] = stencil_src::unquote(&e.value);
                    out.push((Cell0 { name: e.name.clone(), line: e.line, val: CellVal::Text(t) }, 1 << l));
                }
            },
            (None, true) => return Err(err(file, e.line, format!("{0} braucht eine Sprachendung ({0}.de, {0}.en, {0}.es, {0}.it, {0}.fr)", e.name))),
            (Some(_), false) => return Err(err(file, e.line, format!("{}: Sprachendungen gibt es hier nicht", stencil_src::show(e)))),
            (None, false) => {
                let v = stencil_src::classify(&e.value);
                if let Value0::Formula(f) = &v {
                    return Err(err(file, e.line, format!("{}: „{f}“ ist kein Wert (Vorlagen enthalten nur Konstanten: 40 mm, TRUE, \"Text\", #RRGGBB)", e.name)));
                }
                if out.iter().any(|(c, _)| c.name == e.name) {
                    return Err(err(file, e.line, format!("{} doppelt", e.name)));
                }
                out.push((Cell0 { name: e.name.clone(), line: e.line, val: CellVal::Plain(v) }, 0));
            }
        }
    }
    for (c, have) in &out {
        if matches!(c.val, CellVal::Text(_)) && *have != 0b1_1111 {
            let missing = (0..5).find(|i| have & (1 << i) == 0).unwrap_or(0);
            return Err(err(file, c.line, format!("{}.{} fehlt", c.name, LANGS[missing])));
        }
    }
    Ok(out.into_iter().map(|(c, _)| c).collect())
}

fn cell<'c>(cells: &'c [Cell0], name: &str) -> Option<&'c Cell0> {
    cells.iter().find(|c| c.name == name)
}

// a length (a number with a length unit, or 0) in inches
fn length(file: &str, c: &Cell0) -> Result<f64, SourceError> {
    match &c.val {
        CellVal::Plain(Value0::Num(x, Some(u))) if u.dimension() == Dimension::Length => Ok(*x),
        CellVal::Plain(Value0::Num(x, None)) if *x == 0.0 => Ok(0.0),
        _ => Err(err(file, c.line, format!("{} muss eine Länge sein (z. B. 40 mm, 1.5 in)", c.name))),
    }
}

fn string(file: &str, c: &Cell0) -> Result<String, SourceError> {
    match &c.val {
        CellVal::Plain(Value0::Str(s)) if !s.is_empty() => Ok(s.clone()),
        _ => Err(err(file, c.line, format!("{} muss ein Text in Anführungszeichen sein", c.name))),
    }
}

fn boolean(file: &str, c: &Cell0) -> Result<bool, SourceError> {
    match &c.val {
        CellVal::Plain(Value0::Bool(b)) => Ok(*b),
        _ => Err(err(file, c.line, format!("{} muss TRUE oder FALSE sein", c.name))),
    }
}

fn count(file: &str, c: &Cell0, max: usize) -> Result<usize, SourceError> {
    match &c.val {
        CellVal::Plain(Value0::Num(x, None)) if formula::math::round(*x) == *x && *x >= 0.0 && *x <= max as f64 => Ok(*x as usize),
        _ => Err(err(file, c.line, format!("{} muss eine ganze Zahl von 0 bis {max} sein", c.name))),
    }
}

fn texts(c: &Cell0) -> Option<&[String; 5]> {
    match &c.val {
        CellVal::Text(t) => Some(t),
        CellVal::Plain(_) => None,
    }
}

// a constant into a cell
fn put(tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, c: &Cell0, lang: usize, file: &str) -> Result<(), SourceError> {
    let (v, u) = match &c.val {
        CellVal::Plain(Value0::Num(x, u)) => (Value::Num(*x), *u),
        CellVal::Plain(Value0::Bool(b)) => (Value::Num(if *b { 1.0 } else { 0.0 }), Some(Unit::Bool)),
        CellVal::Plain(Value0::Str(s)) => (Value::Text(tx.intern(s)), Some(Unit::Str)),
        CellVal::Plain(Value0::Color(s)) => (Value::Text(tx.intern(s)), None),
        CellVal::Plain(Value0::Formula(f)) => return Err(err(file, c.line, format!("{}: „{f}“ ist kein Wert", c.name))),
        CellVal::Text(t) => (Value::Text(tx.intern(&t[lang])), Some(Unit::Str)),
    };
    tx.force_constant(sheet, addr, v, u).map_err(|e| fail(file, c.line, &c.name, e))
}

fn fail(file: &str, line: u32, what: &str, e: EditError) -> SourceError {
    err(file, line, format!("{what} lässt sich nicht schreiben ({e:?})"))
}

fn user_text(tx: &mut Transaction, sheet: SheetRef, name: &str, value: &str, file: &str, line: u32) -> Result<(), SourceError> {
    let v = tx.intern(value);
    tx.set_user(sheet, name, Value::Text(v), Some(Unit::Str)).map_err(|e| fail(file, line, name, e))
}

// a list of file stems separated by commas
fn stems(file: &str, e: &Entry, key: &str) -> Result<Vec<String>, SourceError> {
    let mut out: Vec<String> = Vec::new();
    for part in e.value.split(',') {
        let p = part.trim();
        if p.is_empty() && e.value.trim().is_empty() {
            break;
        }
        if !stem_ok(p) {
            return Err(err(file, e.line, format!("{key}: „{p}“ ist kein Name (Buchstaben, Ziffern, _ und -)")));
        }
        if out.iter().any(|x| x == p) {
            return Err(err(file, e.line, format!("{key}: {p} doppelt")));
        }
        if out.len() >= MAX_LIST {
            return Err(err(file, e.line, format!("{key}: zu viele")));
        }
        out.push(String::from(p));
    }
    Ok(out)
}

fn stem_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// the first header line must be `key: value` without suffixes
fn first_key<'e>(file: &str, src: &'e TSource, key: &str) -> Result<&'e Entry, SourceError> {
    match src.header.first() {
        Some(e) if e.name == key && e.lang.is_none() && e.variant.is_none() && !e.value.is_empty() => Ok(e),
        Some(e) => Err(err(file, e.line, format!("die erste Kopfzeile muss {key}: Name sein"))),
        None => Err(err(file, 1, format!("die erste Kopfzeile muss {key}: Name sein"))),
    }
}

// ---------------------------------------------------------------- template.txt

struct Head {
    template: String,
    file: String,
    names: [String; 5],
    descriptions: [String; 5],
    category: String,
    order: u32,
    stencils: Vec<String>,
    stencils_line: u32,
    active: usize,
    design: u32,
    variant: u32,
    registers: Vec<String>,
    rules: Option<(String, u32)>,
    starters: Vec<String>,
    starters_line: u32,
    // the starter the catalogue chooses first, as written (None: the blank form) and its line
    default: Option<(String, u32)>,
    blank: bool,
    lang: usize,
    page1: [String; 5],
}

fn head(src: &TSource, m: Measure) -> Result<Head, SourceError> {
    first_key(F, src, "template")?;
    let es = stencil_src::resolve(F, &src.header, m)?;
    stencil::keys_known(
        F,
        &es,
        &["template", "file", "name", "description", "category", "order", "stencils", "active", "design", "variant", "registers", "rules", "starters", "default", "blank", "language", "page1"],
    )?;
    let one = |key: &str| stencil::single(F, &es, key);
    let need = |key: &str| -> Result<&Entry, SourceError> {
        match stencil::single(F, &es, key)? {
            Some(e) if !e.value.is_empty() => Ok(e),
            Some(e) => Err(err(F, e.line, format!("{key}: Wert fehlt"))),
            None => Err(err(F, 1, format!("{key}: fehlt"))),
        }
    };
    let template = need("template")?.value.clone();
    let fe = need("file")?;
    if !stem_ok(&fe.value) {
        return Err(err(F, fe.line, format!("file: „{}“ ist kein Dateistamm (Buchstaben, Ziffern, _ und -)", fe.value)));
    }
    let names = stencil::five(F, &es, "name", true, 1)?.unwrap_or_default();
    let descriptions = stencil::five(F, &es, "description", true, 1)?.unwrap_or_default();
    let ce = need("category")?;
    if !CATEGORIES.contains(&ce.value.as_str()) {
        return Err(err(F, ce.line, format!("category: unbekannte Kategorie {} (bekannt: {})", ce.value, CATEGORIES.join(", "))));
    }
    let order = match one("order")? {
        None => 100,
        Some(e) => e.value.parse::<u32>().map_err(|_| err(F, e.line, format!("order: „{}“ ist keine Zahl", e.value)))?,
    };
    let (stencils, stencils_line) = match one("stencils")? {
        None => (Vec::new(), 1),
        Some(e) => (stems(F, e, "stencils")?, e.line),
    };
    let active = match one("active")? {
        None => 0,
        Some(e) => stencils.iter().position(|s| *s == e.value).ok_or_else(|| err(F, e.line, format!("active: {} steht nicht in stencils", e.value)))?,
    };
    let design = match one("design")? {
        None => design::catalog::NETLAB,
        Some(e) => {
            let all = design::catalog::all();
            let names: Vec<String> = all.iter().map(|d| d.name.clone()).collect();
            all.iter().find(|d| d.name.eq_ignore_ascii_case(&e.value)).map(|d| d.id).ok_or_else(|| err(F, e.line, format!("design: unbekanntes Design {} (bekannt: {})", e.value, names.join(", "))))?
        }
    };
    let variant = match one("variant")? {
        None => 1,
        Some(e) => match e.value.parse::<u32>() {
            Ok(v @ 1..=4) => v,
            _ => return Err(err(F, e.line, format!("variant: „{}“ ist keine Zahl von 1 bis 4", e.value))),
        },
    };
    let registers = match one("registers")? {
        None => Vec::new(),
        Some(e) => {
            let list = stems(F, e, "registers")?;
            if let Some(r) = list.iter().find(|r| !REGISTERS.contains(&r.as_str())) {
                return Err(err(F, e.line, format!("registers: unbekanntes Register {r} (bekannt: {})", REGISTERS.join(", "))));
            }
            list
        }
    };
    let rules = one("rules")?.map(|e| (e.value.clone(), e.line));
    let (starters, starters_line) = match one("starters")? {
        None => (Vec::new(), 1),
        Some(e) => (stems(F, e, "starters")?, e.line),
    };
    let default = one("default")?.filter(|e| !e.value.eq_ignore_ascii_case("blank")).map(|e| (e.value.clone(), e.line));
    let blank = match one("blank")? {
        None => true,
        Some(e) if e.value.eq_ignore_ascii_case("true") => true,
        Some(e) if e.value.eq_ignore_ascii_case("false") => false,
        Some(e) => return Err(err(F, e.line, format!("blank: „{}“ ist weder true noch false", e.value))),
    };
    if !blank && starters.is_empty() {
        return Err(err(F, starters_line, "blank: false braucht Startdiagramme (starters)"));
    }
    if let (false, None) = (blank, &default) {
        return Err(err(F, starters_line, "blank: false braucht default: <Startdiagramm>"));
    }
    let lang = match one("language")? {
        None => 0,
        Some(e) => instance::lang_index(&e.value).ok_or_else(|| err(F, e.line, format!("language: „{}“ ist keine der Sprachen {}", e.value, LANGS.join(", "))))?,
    };
    let page1 = stencil::five(F, &es, "page1", false, 1)?.unwrap_or_else(|| PAGE1.map(String::from));
    Ok(Head {
        template,
        file: fe.value.clone(),
        names,
        descriptions,
        category: ce.value.clone(),
        order,
        stencils,
        stencils_line,
        active,
        design,
        variant,
        registers,
        rules,
        starters,
        starters_line,
        default,
        blank,
        lang,
        page1,
    })
}

// a style's cell: row 0 of Character and Paragraph as Char.<cell> / Para.<cell>, else a single
// cell of a shape
fn style_cell_ok(name: &str) -> bool {
    if let Some(c) = name.strip_prefix("Char.") {
        return stencil::row_cells("Character").contains(&c);
    }
    if let Some(c) = name.strip_prefix("Para.") {
        return stencil::row_cells("Paragraph").contains(&c);
    }
    stencil::shape_cell_known(name)
}

fn style_addr(tx: &mut Transaction, name: &str) -> CellAddr {
    for (prefix, section) in [("Char.", "Character"), ("Para.", "Paragraph")] {
        if let Some(c) = name.strip_prefix(prefix) {
            let s = tx.intern(section);
            let c = tx.intern(c);
            return CellAddr::row(SecKey::one(s), RowKey::Index(0), c);
        }
    }
    tx.single(name)
}

struct PageSrc {
    name_u: String,
    line: u32,
    names: [String; 5],
    background: bool,
    back: Option<(String, u32)>,
}

struct LayerSrc {
    line: u32,
    names: [String; 5],
    cells: Vec<Cell0>,
}

// the pages of [Pages]; a row for Page-1 itself sets only its background page (-> its BackPage)
fn pages_of(src: &TSource, m: Measure) -> Result<(Vec<PageSrc>, Option<(String, u32)>), SourceError> {
    let mut out: Vec<PageSrc> = Vec::new();
    let mut first_back = None;
    for sec in src.sections("Pages") {
        for row in &sec.rows {
            let cells = cells_of(F, &row.cells, m, &|n| n == "Background" || n == "BackPage", "[Pages]", &["Name"])?;
            let back = match cell(&cells, "BackPage") {
                Some(c) => Some((string(F, c)?, c.line)),
                None => None,
            };
            if row.head == "Page-1" {
                if cells.iter().any(|c| c.name != "BackPage") {
                    return Err(err(F, row.line, "Page-1: hier nur BackPage (der Name steht im Kopf: page1.de …)"));
                }
                first_back = back;
                continue;
            }
            if out.iter().any(|p| p.name_u == row.head) {
                return Err(err(F, row.line, format!("Seite {} doppelt", row.head)));
            }
            let names = cell(&cells, "Name").and_then(texts).cloned().ok_or_else(|| err(F, row.line, format!("{}: Name.de … Name.fr fehlen", row.head)))?;
            let background = match cell(&cells, "Background") {
                Some(c) => boolean(F, c)?,
                None => false,
            };
            if out.len() >= MAX_LIST {
                return Err(err(F, row.line, "zu viele Seiten"));
            }
            out.push(PageSrc { name_u: row.head.clone(), line: row.line, names, background, back });
        }
    }
    Ok((out, first_back))
}

fn layers_of(src: &TSource, m: Measure) -> Result<Vec<LayerSrc>, SourceError> {
    let mut out = Vec::new();
    for sec in src.sections("Layer") {
        for row in &sec.rows {
            let cells = cells_of(F, &row.cells, m, &|n| LAYER_FLAGS.contains(&n) || n == "Color", "[Layer]", &["Name"])?;
            let names = cell(&cells, "Name").and_then(texts).cloned().ok_or_else(|| err(F, row.line, "Name.de … Name.fr fehlen"))?;
            for c in &cells {
                match c.name.as_str() {
                    "Name" => {}
                    "Color" => {
                        count(F, c, 255)?;
                    }
                    _ => {
                        boolean(F, c)?;
                    }
                }
            }
            if out.len() >= MAX_LIST {
                return Err(err(F, row.line, "zu viele Ebenen"));
            }
            out.push(LayerSrc { line: row.line, names, cells });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- rules.txt

struct RuleSrc {
    name_u: String,
    categories: [String; 5],
    descriptions: [String; 5],
    target: u32,
    filter: String,
    test: String,
}

struct RuleSet {
    name_u: String,
    names: [String; 5],
    descriptions: [String; 5],
    rules: Vec<RuleSrc>,
}

fn rules_of(file: &str, text: &str, m: Measure) -> Result<RuleSet, SourceError> {
    let src = template_src::parse(file, text, RULES_SECS)?;
    first_key(file, &src, "ruleset")?;
    let es = stencil_src::resolve(file, &src.header, m)?;
    stencil::keys_known(file, &es, &["ruleset", "name", "description"])?;
    let name_u = stencil::single(file, &es, "ruleset")?.map(|e| e.value.clone()).unwrap_or_default();
    if name_u.contains(['.', '|']) {
        return Err(err(file, 1, "ruleset: der Name darf weder . noch | enthalten"));
    }
    let names = stencil::five(file, &es, "name", true, 1)?.unwrap_or_default();
    let descriptions = stencil::five(file, &es, "description", true, 1)?.unwrap_or_default();
    let mut rules: Vec<RuleSrc> = Vec::new();
    for sec in src.sections("Rules") {
        for row in &sec.rows {
            if row.head.contains(['.', '|']) {
                return Err(err(file, row.line, format!("Regel „{}“: der Name darf weder . noch | enthalten", row.head)));
            }
            let cells = cells_of(file, &row.cells, m, &|n| matches!(n, "Target" | "Filter" | "Test"), "[Rules]", &["Category", "Description"])?;
            let need5 = |n: &str| cell(&cells, n).and_then(texts).cloned().ok_or_else(|| err(file, row.line, format!("{}: {n}.de … {n}.fr fehlen", row.head)));
            let categories = need5("Category")?;
            let descriptions = need5("Description")?;
            let target = match cell(&cells, "Target") {
                Some(c) => count(file, c, 2)? as u32,
                None => return Err(err(file, row.line, format!("{}: Target fehlt (0 Shape, 1 Seite, 2 Dokument)", row.head))),
            };
            let formula = |n: &str| -> Result<String, SourceError> {
                let c = cell(&cells, n).ok_or_else(|| err(file, row.line, format!("{}: {n} fehlt", row.head)))?;
                let f = string(file, c)?;
                formula::parse(&f).map_err(|e| err(file, c.line, format!("{n}: Formel nicht lesbar ab Zeichen {}: {f}", e.pos + 1)))?;
                Ok(f)
            };
            let (filter, test) = (formula("Filter")?, formula("Test")?);
            if rules.len() >= MAX_LIST {
                return Err(err(file, row.line, "zu viele Regeln"));
            }
            rules.push(RuleSrc { name_u: row.head.clone(), categories, descriptions, target, filter, test });
        }
    }
    if rules.is_empty() {
        return Err(err(file, 0, "der Regelsatz hat keine Regeln ([Rules])"));
    }
    Ok(RuleSet { name_u, names, descriptions, rules })
}

fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("='");
    escape(out, value, true);
    out.push('\'');
}

// the rule set's part in language `lang`
fn validation_xml(rs: &RuleSet, lang: usize) -> Vec<u8> {
    let mut s = String::from(DECL);
    s.push_str("<Validation");
    attr(&mut s, "xmlns", NS_VISIO);
    attr(&mut s, "xmlns:r", NS_REL);
    attr(&mut s, "xml:space", "preserve");
    s.push_str("><ValidationProperties ShowIgnored='0'/><RuleSets><RuleSet ID='1'");
    attr(&mut s, "NameU", &rs.name_u);
    attr(&mut s, "Name", &rs.names[lang]);
    attr(&mut s, "Description", &rs.descriptions[lang]);
    s.push_str("><RuleSetFlags Hidden='0'/>");
    for (k, r) in rs.rules.iter().enumerate() {
        s.push_str(&format!("<Rule ID='{}'", k + 1));
        attr(&mut s, "NameU", &r.name_u);
        attr(&mut s, "Category", &r.categories[lang]);
        attr(&mut s, "Description", &r.descriptions[lang]);
        s.push_str(&format!(" RuleTarget='{}'><RuleFilter>", r.target));
        escape(&mut s, &r.filter, false);
        s.push_str("</RuleFilter><RuleTest>");
        escape(&mut s, &r.test, false);
        s.push_str("</RuleTest></Rule>");
    }
    s.push_str("</RuleSet></RuleSets></Validation>");
    s.into_bytes()
}

// ---------------------------------------------------------------- *.starter

enum ShapeKind0 {
    Master { stencil: String, master: String },
    // the rows (indexes into the starter's shapes) of both ends
    Connector { from: usize, to: usize },
    Cff { horizontal: bool, lanes: usize, phases: usize },
}

struct ShapeSrc {
    line: u32,
    kind: ShapeKind0,
    pin: [f64; 2],
    size: [Option<(f64, Option<Unit>)>; 2],
    angle: Option<(f64, Option<Unit>)>,
    text: Option<[String; 5]>,
    tip: bool,
    // (JD26) a 1-D master placed by its ends: BeginX, BeginY, EndX, EndY (inches)
    ends: Option<[f64; 4]>,
    // (JD26) the ends glued to a connection point: (row index of the target, point from 0, line)
    glued: [Option<(usize, u32, u32)>; 2],
}

struct Starter {
    file: String,
    // the universal name (`starter:`)
    name: String,
    names: [String; 5],
    description: Option<[String; 5]>,
    shapes: Vec<ShapeSrc>,
}

const SHAPE_CELLS: &[&str] = &[
    "Stencil", "Master", "PinX", "PinY", "Width", "Height", "Angle", "Tip", "From", "To", "Cff", "Lanes", "Phases", "BeginX", "BeginY", "EndX", "EndY",
    "BeginOn", "BeginPoint", "EndOn", "EndPoint",
];

fn starter_of(file: &str, text: &str, m: Measure) -> Result<Starter, SourceError> {
    let src = template_src::parse(file, text, STARTER_SECS)?;
    first_key(file, &src, "starter")?;
    let es = stencil_src::resolve(file, &src.header, m)?;
    stencil::keys_known(file, &es, &["starter", "name", "description"])?;
    let name = stencil::single(file, &es, "starter")?.map(|e| e.value.clone()).unwrap_or_default();
    if !stem_ok(&name) {
        return Err(err(file, 1, format!("starter: „{name}“ ist kein Name für die Datei (Buchstaben, Ziffern, _ und -)")));
    }
    let names = stencil::five(file, &es, "name", true, 1)?.unwrap_or_default();
    let description = stencil::five(file, &es, "description", false, 1)?;
    let mut heads: Vec<(String, bool)> = Vec::new();
    let mut shapes: Vec<ShapeSrc> = Vec::new();
    for sec in src.sections("Shapes") {
        for row in &sec.rows {
            if shapes.len() >= MAX_SHAPES {
                return Err(err(file, row.line, "zu viele Shapes"));
            }
            let cells = cells_of(file, &row.cells, m, &|n| SHAPE_CELLS.contains(&n), "[Shapes]", &["Text"])?;
            let get = |n: &str| cell(&cells, n);
            let only = |allowed: &[&str], what: &str| -> Result<(), SourceError> {
                match cells.iter().find(|c| !allowed.contains(&c.name.as_str())) {
                    Some(c) => Err(err(file, c.line, format!("{}: {} gibt es bei {what} nicht", row.head, c.name))),
                    None => Ok(()),
                }
            };
            let pin = |cells: &[Cell0]| -> Result<[f64; 2], SourceError> {
                let x = cell(cells, "PinX").ok_or_else(|| err(file, row.line, format!("{}: PinX fehlt", row.head)))?;
                let y = cell(cells, "PinY").ok_or_else(|| err(file, row.line, format!("{}: PinY fehlt", row.head)))?;
                Ok([length(file, x)?, length(file, y)?])
            };
            let text = get("Text").and_then(texts).cloned();
            let row_of = |n: &str| -> Result<usize, SourceError> {
                let c = get(n).ok_or_else(|| err(file, row.line, format!("{}: {n} fehlt", row.head)))?;
                let target = string(file, c)?;
                match heads.iter().position(|(h, _)| *h == target) {
                    Some(i) if !heads[i].1 => Ok(i),
                    Some(_) => Err(err(file, c.line, format!("{n}: {target} ist ein Verbinder"))),
                    None => Err(err(file, c.line, format!("{n}: es gibt keine Zeile {target} davor"))),
                }
            };
            let (kind, shape) = if get("From").is_some() || get("To").is_some() {
                only(&["From", "To", "Text"], "einem Verbinder (From, To)")?;
                let (from, to) = (row_of("From")?, row_of("To")?);
                let s = ShapeSrc { line: row.line, kind: ShapeKind0::Connector { from, to }, pin: [0.0; 2], size: [None, None], angle: None, text, tip: false, ends: None, glued: [None, None] };
                (true, s)
            } else if let Some(c) = get("Cff") {
                only(&["Cff", "PinX", "PinY", "Lanes", "Phases", "Text"], "einem funktionsübergreifenden Flussdiagramm (Cff)")?;
                let horizontal = match string(file, c)?.as_str() {
                    "horizontal" => true,
                    "vertical" => false,
                    o => return Err(err(file, c.line, format!("Cff: „{o}“ ist weder \"horizontal\" noch \"vertical\""))),
                };
                let lanes = get("Lanes").map_or(Ok(2), |c| count(file, c, MAX_LANES))?.max(1);
                let phases = get("Phases").map_or(Ok(1), |c| count(file, c, MAX_LANES))?;
                let s = ShapeSrc { line: row.line, kind: ShapeKind0::Cff { horizontal, lanes, phases }, pin: pin(&cells)?, size: [None, None], angle: None, text, tip: false, ends: None, glued: [None, None] };
                (false, s)
            } else {
                only(
                    &["Stencil", "Master", "PinX", "PinY", "Width", "Height", "Angle", "Text", "Tip", "BeginX", "BeginY", "EndX", "EndY", "BeginOn", "BeginPoint", "EndOn", "EndPoint"],
                    "einem Shape",
                )?;
                let st = get("Stencil").ok_or_else(|| err(file, row.line, format!("{}: Stencil und Master fehlen (oder From/To, Cff)", row.head)))?;
                let ms = get("Master").ok_or_else(|| err(file, row.line, format!("{}: Master fehlt", row.head)))?;
                let size_of = |n: &str| -> Result<Option<(f64, Option<Unit>)>, SourceError> {
                    match get(n) {
                        None => Ok(None),
                        Some(c) => {
                            let v = length(file, c)?;
                            if v <= 0.0 {
                                return Err(err(file, c.line, format!("{n} muss positiv sein")));
                            }
                            let u = match &c.val {
                                CellVal::Plain(Value0::Num(_, u)) => *u,
                                _ => None,
                            };
                            Ok(Some((v, u)))
                        }
                    }
                };
                let angle = match get("Angle") {
                    None => None,
                    Some(c) => match &c.val {
                        CellVal::Plain(Value0::Num(x, Some(u))) if u.dimension() == Dimension::Angle => Some((*x, Some(*u))),
                        CellVal::Plain(Value0::Num(x, None)) if *x == 0.0 => Some((0.0, None)),
                        _ => return Err(err(file, c.line, "Angle muss ein Winkel sein (z. B. 90 deg)")),
                    },
                };
                let tip = match get("Tip") {
                    Some(c) => boolean(file, c)?,
                    None => false,
                };
                let kind = ShapeKind0::Master { stencil: string(file, st)?, master: string(file, ms)? };
                // (JD26) a 1-D master by its ends instead of its pin, each end glued to a
                // connection point of a shape in an earlier row (BeginOn / BeginPoint, from 1)
                let ends = match ["BeginX", "BeginY", "EndX", "EndY"].map(get) {
                    [None, None, None, None] => None,
                    [Some(bx), Some(by), Some(ex), Some(ey)] => {
                        if let Some(c) = get("PinX").or(get("PinY")).or(get("Width")).or(get("Angle")) {
                            return Err(err(file, c.line, format!("{}: {} gibt es neben BeginX … EndY nicht", row.head, c.name)));
                        }
                        Some([length(file, bx)?, length(file, by)?, length(file, ex)?, length(file, ey)?])
                    }
                    _ => return Err(err(file, row.line, format!("{}: BeginX, BeginY, EndX und EndY gehören zusammen", row.head))),
                };
                let mut glued = [None, None];
                for (k, (on, at)) in [("BeginOn", "BeginPoint"), ("EndOn", "EndPoint")].into_iter().enumerate() {
                    match (get(on), get(at)) {
                        (None, None) => {}
                        (Some(o), Some(p)) => {
                            if ends.is_none() {
                                return Err(err(file, o.line, format!("{}: {on} braucht BeginX … EndY", row.head)));
                            }
                            let n = count(file, p, 10_000)?;
                            if n == 0 {
                                return Err(err(file, p.line, format!("{at} zählt ab 1")));
                            }
                            glued[k] = Some((row_of(on)?, n as u32 - 1, o.line));
                        }
                        (Some(c), None) | (None, Some(c)) => return Err(err(file, c.line, format!("{}: {on} und {at} gehören zusammen", row.head))),
                    }
                }
                let pin = match ends {
                    Some(e) => [(e[0] + e[2]) * 0.5, (e[1] + e[3]) * 0.5],
                    None => pin(&cells)?,
                };
                let s = ShapeSrc { line: row.line, kind, pin, size: [size_of("Width")?, size_of("Height")?], angle, text, tip, ends, glued };
                (false, s)
            };
            heads.push((row.head.clone(), kind));
            shapes.push(shape);
        }
    }
    Ok(Starter { file: String::from(file), name, names, description, shapes })
}

// ---------------------------------------------------------------- building

// the stencils asked for so far (each built once per measure)
struct Stencils<'a> {
    get: &'a dyn Fn(&str) -> Option<Document>,
    have: Vec<(String, Option<Document>)>,
}

impl Stencils<'_> {
    fn doc(&mut self, stem: &str) -> Option<&Document> {
        if !self.have.iter().any(|(s, _)| s == stem) {
            let d = (self.get)(stem);
            self.have.push((String::from(stem), d));
        }
        self.have.iter().find(|(s, _)| s == stem).and_then(|(_, d)| d.as_ref())
    }
}

fn stencil_file(stem: &str, m: Measure) -> String {
    crate::stencil::file_name(stem, m)
}

// the template every form starts from (step 2 of the module comment)
fn base_doc(h: &Head, src: &TSource, m: Measure, rules: Option<&RuleSet>) -> Result<Document, SourceError> {
    let lang = h.lang;
    let mut doc = vsdx::new_document(DocKind::Template, m);
    let p1 = doc.pages().next().ok_or_else(|| err(F, 0, "die neue Vorlage hat keine Seite"))?;
    let page_cells = match src.sections("PageSheet").next() {
        Some(sec) => cells_of(F, &sec.cells, m, &|n| PAGE_CELLS.contains(&n), "[PageSheet]", &[])?,
        None => Vec::new(),
    };
    let (pages, first_back) = pages_of(src, m)?;
    let layers = layers_of(src, m)?;
    {
        let mut tx = doc.transaction("Vorlage");
        for sec in src.sections("Style") {
            let r = tx.doc().style_by_name_u(&sec.arg).ok_or_else(|| err(F, sec.line, format!("Formatvorlage „{}“ gibt es nicht (No Style, Theme, Normal, Connector, Guide, Text Only)", sec.arg)))?;
            let cells = cells_of(F, &sec.cells, m, &style_cell_ok, &format!("[Style {}]", sec.arg), &[])?;
            for c in &cells {
                let addr = style_addr(&mut tx, &c.name);
                put(&mut tx, SheetRef::Style(r), &addr, c, lang, F)?;
            }
        }
        tx.rename_page(p1, &h.page1[lang], "Page-1").map_err(|e| fail(F, 1, "page1", e))?;
        let ps = SheetRef::Page(p1);
        for c in &page_cells {
            let addr = tx.single(&c.name);
            put(&mut tx, ps, &addr, c, lang, F)?;
        }
        // the orientation follows a size given without it
        let sized = page_cells.iter().any(|c| c.name == "PageWidth" || c.name == "PageHeight");
        if sized && cell(&page_cells, "PrintPageOrientation").is_none() {
            let num = |tx: &mut Transaction, n: &str| {
                let a = tx.single(n);
                tx.doc().resolve(ps, &a).num().unwrap_or(0.0)
            };
            let (w, hh) = (num(&mut tx, "PageWidth"), num(&mut tx, "PageHeight"));
            let a = tx.single("PrintPageOrientation");
            tx.force_constant(ps, &a, Value::Num(if hh > w { 1.0 } else { 2.0 }), None).map_err(|e| fail(F, 1, "PrintPageOrientation", e))?;
        }
        let _ = tx.commit();
    }
    // the other pages: their page sheet a copy of the first one's
    let sheet1 = doc.page(p1).map(|p| p.sheet.clone()).unwrap_or_default();
    let mut refs: Vec<PageRef> = Vec::with_capacity(pages.len());
    {
        let mut tx = doc.transaction("Seiten");
        for p in &pages {
            refs.push(tx.add_page(&p.names[lang], &p.name_u, p.background, None).map_err(|e| fail(F, p.line, &p.name_u, e))?);
        }
        let _ = tx.commit();
    }
    for &r in &refs {
        if let Some(pg) = doc.page_mut(r) {
            pg.sheet = sheet1.clone();
            pg.header.sheet_styles = StyleRefs::all(0);
        }
    }
    let mut eng = Engine::new();
    {
        let mut tx = doc.transaction("Vorlage");
        // the page names in five languages, backgrounds, layers
        user_text(&mut tx, SheetRef::Page(p1), instance::JFD_NAME, &join5(&h.page1), F, 1)?;
        for (p, &r) in pages.iter().zip(&refs) {
            user_text(&mut tx, SheetRef::Page(r), instance::JFD_NAME, &join5(&p.names), F, p.line)?;
        }
        let backs = core::iter::once((p1, first_back.as_ref())).chain(refs.iter().zip(&pages).map(|(&r, p)| (r, p.back.as_ref())));
        let backs: Vec<(PageRef, Option<&(String, u32)>)> = backs.collect();
        for (r, back) in backs {
            let Some((name, line)) = back else { continue };
            let b = tx.doc().page_by_name_u(name).ok_or_else(|| err(F, *line, format!("BackPage: die Seite {name} gibt es nicht")))?;
            tx.set_background(r, Some(b)).map_err(|e| err(F, *line, format!("BackPage: {name} ist keine Hintergrundseite ({e:?})")))?;
        }
        let fg: Vec<PageRef> = tx.doc().foreground_pages().collect();
        for &p in &fg {
            for l in &layers {
                let ix = tx.add_layer(p, &l.names[lang], &l.names[1]).map_err(|e| fail(F, l.line, "die Ebene", e))?;
                let sk = SecKey::one(tx.intern("Layer"));
                for c in l.cells.iter().filter(|c| c.name != "Name") {
                    let a = CellAddr::row(sk, RowKey::Index(ix), tx.intern(&c.name));
                    let v = match &c.val {
                        CellVal::Plain(Value0::Bool(b)) => Value::Num(if *b { 1.0 } else { 0.0 }),
                        CellVal::Plain(Value0::Num(x, _)) => Value::Num(*x),
                        _ => Value::Num(0.0),
                    };
                    tx.force_constant(SheetRef::Page(p), &a, v, None).map_err(|e| fail(F, c.line, &c.name, e))?;
                }
                user_text(&mut tx, SheetRef::Page(p), &format!("{JFD_LAYER}{}", ix + 1), &join5(&l.names), F, l.line)?;
            }
        }
        // the catalogue's cells
        let ds = SheetRef::Document;
        user_text(&mut tx, ds, JFD_TEMPLATE, &h.template, F, 1)?;
        user_text(&mut tx, ds, instance::JFD_NAME, &join5(&h.names), F, 1)?;
        user_text(&mut tx, ds, JFD_DESCRIPTION, &join5(&h.descriptions), F, 1)?;
        user_text(&mut tx, ds, instance::JFD_CATEGORY, &h.category, F, 1)?;
        tx.set_user(ds, JFD_ORDER, Value::Num(h.order as f64), None).map_err(|e| fail(F, 1, JFD_ORDER, e))?;
        if !h.registers.is_empty() {
            user_text(&mut tx, ds, JFD_REGISTERS, &h.registers.join(";"), F, 1)?;
        }
        if let Some(rs) = rules {
            let mut f: Vec<&str> = alloc::vec![rs.name_u.as_str()];
            f.extend(rs.names.iter().map(String::as_str));
            f.extend(rs.descriptions.iter().map(String::as_str));
            user_text(&mut tx, ds, &format!("{JFD_RULE_SET}1"), &join_fields(&f), F, 1)?;
            for (k, r) in rs.rules.iter().enumerate() {
                let key = format!("{}.{}", rs.name_u, r.name_u);
                let mut f: Vec<&str> = alloc::vec![key.as_str()];
                f.extend(r.descriptions.iter().map(String::as_str));
                f.extend(r.categories.iter().map(String::as_str));
                user_text(&mut tx, ds, &format!("{JFD_RULE}{}", k + 1), &join_fields(&f), F, 1)?;
            }
        }
        // the design on every page
        let d = design::catalog::by_id(h.design).ok_or_else(|| err(F, 1, "design: unbekannt"))?;
        design::apply::apply(&mut tx, &mut eng, d.id, &d.theme_xml(), Scope::All, h.variant - 1).map_err(|e| fail(F, 1, "das Design", e))?;
        let _ = tx.commit();
    }
    let list: Vec<StencilWindow> = h.stencils.iter().enumerate().map(|(k, s)| StencilWindow { document: stencil_file(s, m), active: k == h.active }).collect();
    set_stencil_windows(&mut doc, &list);
    if let Some(rs) = rules {
        let name = doc.intern("/visio/validation.xml");
        let ct = doc.intern(CT_VALIDATION);
        let rel = doc.intern(REL_VALIDATION);
        doc.parts.push(Part { name, content_type: ct, rel_type: rel, data: validation_xml(rs, lang) });
    }
    Ok(doc)
}

// the sheets of a shape and its subshapes
fn sheets_of(doc: &Document, s: ShapeRef) -> Vec<SheetRef> {
    let mut out = Vec::new();
    let mut stack = alloc::vec![s];
    while let Some(c) = stack.pop() {
        out.push(SheetRef::Shape(c));
        if let Some(sh) = doc.shape(c) {
            stack.extend(sh.children());
        }
    }
    out
}

// (JD26) whether a document measures in millimetres (the first page's PageWidth unit)
fn doc_metric(doc: &Document) -> bool {
    let Some(page) = doc.pages().next() else { return true };
    let Some(sym) = doc.sym("PageWidth") else { return true };
    doc.resolve(SheetRef::Page(page), &CellAddr::single(sym)).unit() != Some(Unit::Inches)
}

// a text in five languages on a shape: the build language's as its text, all in JFD_Text
fn shape_text(tx: &mut Transaction, s: ShapeRef, t: &[String; 5], lang: usize, file: &str, line: u32) -> Result<(), SourceError> {
    tx.set_text(s, Some(Text::plain(&t[lang]))).map_err(|e| fail(file, line, "der Text", e))?;
    user_text(tx, SheetRef::Shape(s), instance::JFD_TEXT, &join5(t), file, line)
}

// the shapes of a starter diagram on the first page (step 3 of the module comment)
fn place_starter(doc: &mut Document, st: &Starter, lang: usize, stencils: &mut Stencils) -> Result<(), SourceError> {
    let file = st.file.as_str();
    let page = doc.foreground_pages().next().ok_or_else(|| err(file, 0, "die Vorlage hat keine Vordergrundseite"))?;
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let mut made: Vec<ShapeRef> = Vec::with_capacity(st.shapes.len());
    let mut tx = doc.transaction("Startdiagramm");
    for s in &st.shapes {
        let line = s.line;
        let shape = match &s.kind {
            ShapeKind0::Master { stencil, master } => {
                let src = stencils.doc(stencil).ok_or_else(|| err(file, line, format!("Stencil: die Schablone {stencil} gibt es nicht")))?;
                let m = src.master_by_name_u(master).ok_or_else(|| err(file, line, format!("Master: {stencil} hat keinen Master „{master}“")))?;
                let im = instance::import_master(&mut tx, src, m).map_err(|e| fail(file, line, "der Master", e))?;
                let shape = instance::place(&mut tx, page, im, s.pin[0], s.pin[1], lang).map_err(|e| fail(file, line, "das Shape", e))?;
                for (name, v) in [("Width", s.size[0]), ("Height", s.size[1]), ("Angle", s.angle)] {
                    if let Some((x, u)) = v {
                        let a = tx.single(name);
                        tx.force_constant(SheetRef::Shape(shape), &a, Value::Num(x), u).map_err(|e| fail(file, line, name, e))?;
                    }
                }
                if let Some(t) = &s.text {
                    shape_text(&mut tx, shape, t, lang, file, line)?;
                }
                if s.tip {
                    tx.set_user(SheetRef::Shape(shape), JFD_TIP, Value::Num(1.0), Some(Unit::Bool)).map_err(|e| fail(file, line, JFD_TIP, e))?;
                }
                // (JD26) a 1-D master by its ends: the ends as written (the pin follows them)
                if let Some(e) = s.ends {
                    let u = if doc_metric(tx.doc()) { Unit::Millimeters } else { Unit::Inches };
                    for (name, v) in [("BeginX", e[0]), ("BeginY", e[1]), ("EndX", e[2]), ("EndY", e[3])] {
                        let a = tx.single(name);
                        tx.force_constant(SheetRef::Shape(shape), &a, Value::Num(v), Some(u)).map_err(|e| fail(file, line, name, e))?;
                    }
                }
                let sheets = sheets_of(tx.doc(), shape);
                eng.recalc(&mut tx, &sheets).map_err(|e| fail(file, line, "die Formeln", e))?;
                // (JD26) and glued to the connection points named
                for (k, g) in s.glued.iter().enumerate() {
                    let Some((row, point, gline)) = *g else { continue };
                    let Some(&target) = made.get(row) else { return Err(err(file, gline, "die Zeile fehlt")) };
                    let at = glue::connection_points(tx.doc(), target).into_iter().find(|p| p.row == point).map(|p| p.at);
                    let at = at.ok_or_else(|| err(file, gline, format!("das Ziel hat keinen Verbindungspunkt {}", point + 1)))?;
                    let end = if k == 0 { glue::End::Begin } else { glue::End::End };
                    glue::glue(&mut tx, &mut eng, shape, end, at, Target::Point { shape: target, row: point }).map_err(|e| fail(file, gline, "das Kleben", e))?;
                }
                shape
            }
            ShapeKind0::Connector { from, to } => {
                let (Some(&a), Some(&b)) = (made.get(*from), made.get(*to)) else { return Err(err(file, line, "From/To: die Zeile fehlt")) };
                let (pa, pb) = {
                    let d = tx.doc();
                    (diagram_core::ops::frame(d, a).centre(), diagram_core::ops::frame(d, b).centre())
                };
                let c = glue::new_connector(&mut tx, &mut eng, owner, (pa, Target::Shape(a)), (pb, Target::Shape(b))).map_err(|e| fail(file, line, "der Verbinder", e))?;
                if let Some(t) = &s.text {
                    shape_text(&mut tx, c, t, lang, file, line)?;
                    eng.recalc(&mut tx, &[SheetRef::Shape(c)]).map_err(|e| fail(file, line, "die Formeln", e))?;
                }
                glue::update(&mut tx, &mut eng, &[c]).map_err(|e| fail(file, line, "der Verbinder", e))?;
                c
            }
            ShapeKind0::Cff { horizontal, lanes, phases } => {
                let src = stencils.doc("cff").ok_or_else(|| err(file, line, "Cff: die Schablone cff gibt es nicht"))?;
                let f = cff::create(&mut tx, &mut eng, src, page, s.pin, *horizontal, *lanes, *phases, lang).map_err(|e| fail(file, line, "das Flussdiagramm", e))?;
                if let Some(t) = &s.text {
                    shape_text(&mut tx, f.pool, t, lang, file, line)?;
                    let sheets = sheets_of(tx.doc(), f.pool);
                    eng.recalc(&mut tx, &sheets).map_err(|e| fail(file, line, "die Formeln", e))?;
                }
                f.pool
            }
        };
        made.push(shape);
    }
    let _ = tx.commit();
    Ok(())
}

// every formula computed and stored (step 4)
fn compute(doc: &mut Document, file: &str) -> Result<(), SourceError> {
    let res = Engine::new().calculate(doc);
    let mut tx = doc.transaction("Werte");
    res.write(&mut tx).map_err(|e| err(file, 0, format!("Formelergebnisse lassen sich nicht schreiben ({e:?})")))?;
    let _ = tx.commit();
    Ok(())
}

/// build every form of the template of one folder for one measure (see the module comment)
pub fn build(files: &dyn Fn(&str) -> Option<String>, measure: Measure, stencils: &dyn Fn(&str) -> Option<Document>) -> Result<Vec<BuiltTemplate>, SourceError> {
    let text = files(F).ok_or_else(|| err(F, 0, "template.txt fehlt"))?;
    let src = template_src::parse(F, &text, TEMPLATE_SECS)?;
    let h = head(&src, measure)?;
    let mut cache = Stencils { get: stencils, have: Vec::new() };
    for stem in &h.stencils {
        if cache.doc(stem).is_none() {
            return Err(err(F, h.stencils_line, format!("stencils: die Schablone {stem} gibt es nicht (der Dateistamm aus ihrer stencil.txt)")));
        }
    }
    let rules = match &h.rules {
        None => None,
        Some((name, line)) => {
            let t = files(name).ok_or_else(|| err(F, *line, format!("rules: die Datei {name} fehlt")))?;
            Some(rules_of(name, &t, measure)?)
        }
    };
    let mut starters: Vec<Starter> = Vec::with_capacity(h.starters.len());
    for stem in &h.starters {
        let file = format!("{stem}.starter");
        let t = files(&file).ok_or_else(|| err(F, h.starters_line, format!("starters: die Datei {file} fehlt")))?;
        let st = starter_of(&file, &t, measure)?;
        if let Some(prev) = starters.iter().find(|p| p.name == st.name) {
            return Err(err(&file, 1, format!("starter: {} hat schon {}", st.name, prev.file)));
        }
        starters.push(st);
    }
    let default = match &h.default {
        None => None,
        Some((name, line)) => Some(
            h.starters.iter().zip(&starters).position(|(stem, st)| stem == name || st.name == *name).ok_or_else(|| err(F, *line, format!("default: {name} ist weder blank noch ein Startdiagramm")))?,
        ),
    };
    let base = base_doc(&h, &src, measure, rules.as_ref())?;
    let ds = SheetRef::Document;
    let mut out = Vec::with_capacity(1 + starters.len());
    // the blank form
    let mut doc = base.clone();
    if default.is_none() {
        let mut tx = doc.transaction("Vorlage");
        tx.set_user(ds, JFD_DEFAULT, Value::Num(1.0), None).map_err(|e| fail(F, 1, JFD_DEFAULT, e))?;
        let _ = tx.commit();
    }
    compute(&mut doc, F)?;
    let built = |doc: Document, file: String, starter: Option<String>, listed: bool| BuiltTemplate {
        doc,
        template: h.template.clone(),
        file,
        starter,
        category: h.category.clone(),
        order: h.order,
        measure,
        listed,
    };
    out.push(built(doc, h.file.clone(), None, h.blank));
    // the starter diagrams
    for (k, st) in starters.iter().enumerate() {
        let mut doc = base.clone();
        {
            let mut tx = doc.transaction("Startdiagramm");
            user_text(&mut tx, ds, JFD_STARTER, &join5(&st.names), &st.file, 1)?;
            if let Some(d) = &st.description {
                user_text(&mut tx, ds, JFD_DESCRIPTION, &join5(d), &st.file, 1)?;
            }
            if default == Some(k) {
                tx.set_user(ds, JFD_DEFAULT, Value::Num(1.0), None).map_err(|e| fail(F, 1, JFD_DEFAULT, e))?;
            }
            let _ = tx.commit();
        }
        place_starter(&mut doc, st, h.lang, &mut cache)?;
        compute(&mut doc, &st.file)?;
        out.push(built(doc, format!("{}-{}", h.file, st.name), Some(st.name.clone()), true));
    }
    Ok(out)
}

/// build every form and write each as .vstx bytes (vsdx::write with the default options)
pub fn build_vstx(files: &dyn Fn(&str) -> Option<String>, measure: Measure, stencils: &dyn Fn(&str) -> Option<Document>) -> Result<Vec<(BuiltTemplate, Vec<u8>)>, SourceError> {
    let mut out = Vec::new();
    for b in build(files, measure, stencils)? {
        let bytes = vsdx::write(&b.doc, &WriteOptions::DEFAULT).map_err(|e| err(F, 0, format!("{} lässt sich nicht schreiben: {e}", b.vstx_name())))?;
        out.push((b, bytes));
    }
    Ok(out)
}

// ---------------------------------------------------------------- the windows part

// a child of the Windows element as read
struct Win {
    // its text as written
    raw: String,
    id: Option<u32>,
    kind: String,
    state: u32,
    document: String,
    pos: Option<i64>,
}

struct WindowsXml {
    // the root's attributes (qualified name, value)
    root: Vec<(String, String)>,
    children: Vec<Win>,
}

fn windows_part(doc: &Document) -> Option<usize> {
    doc.parts.iter().position(|p| doc.str(p.content_type) == CT_WINDOWS)
}

fn parse_windows(data: &[u8]) -> Option<WindowsXml> {
    let text = xml::text_of(data)?;
    let mut r = Reader::new(&text, 64, 256);
    let root = loop {
        match r.next().ok()? {
            Event::Start(s) => break s,
            Event::Eof => return None,
            _ => {}
        }
    };
    if root.local != "Windows" {
        return None;
    }
    let mut out = WindowsXml { root: root.attrs.iter().map(|a| (String::from(a.qname), a.value.to_string())).collect(), children: Vec::new() };
    if root.empty {
        return Some(out);
    }
    loop {
        match r.next().ok()? {
            Event::Start(s) => {
                let get = |n: &str| s.attrs.iter().find(|a| a.ns != xml::Ns::Decl && a.local == n).map(|a| a.value.to_string());
                let mut w = Win {
                    raw: String::new(),
                    id: get("ID").and_then(|v| v.trim().parse().ok()),
                    kind: if s.local == "Window" { get("WindowType").unwrap_or_default() } else { String::new() },
                    state: get("WindowState").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                    document: get("Document").unwrap_or_default(),
                    pos: None,
                };
                // its children: StencilGroupPos; the end gives its text as written
                let mut depth = 0usize;
                let mut in_pos = false;
                let end = loop {
                    match r.next().ok()? {
                        Event::Start(c) => {
                            depth += 1;
                            in_pos = depth == 1 && c.local == "StencilGroupPos";
                        }
                        Event::Text(t) if in_pos => w.pos = t.trim().parse().ok(),
                        Event::Text(_) => {}
                        Event::End { end } => {
                            if depth == 0 {
                                break end;
                            }
                            depth -= 1;
                            in_pos = false;
                        }
                        Event::Eof => return None,
                    }
                };
                w.raw = String::from(text.get(s.at..end)?);
                if out.children.len() >= 100_000 {
                    return None;
                }
                out.children.push(w);
            }
            Event::End { .. } => return Some(out),
            Event::Text(_) => {}
            Event::Eof => return None,
        }
    }
}

/// the stencils the document's windows part opens, by StencilGroupPos, else in document order
/// (see the module comment); none without a part or a readable one
pub fn stencil_windows(doc: &Document) -> Vec<StencilWindow> {
    let Some(w) = windows_part(doc).and_then(|i| parse_windows(&doc.parts[i].data)) else { return Vec::new() };
    let mut list: Vec<&Win> = w.children.iter().filter(|c| c.kind.eq_ignore_ascii_case("Stencil") && !c.document.trim().is_empty()).collect();
    list.sort_by_key(|c| (c.pos.is_none(), c.pos.unwrap_or(0)));
    list.into_iter().map(|c| StencilWindow { document: c.document.clone(), active: c.state & WS_ACTIVE != 0 }).collect()
}

/// set the windows part to these stencils, in this order (see the module comment): the part's
/// other windows stay, its stencil windows are replaced; a part of its own when there is none
pub fn set_stencil_windows(doc: &mut Document, list: &[StencilWindow]) {
    let at = windows_part(doc);
    let parsed = at.and_then(|i| parse_windows(&doc.parts[i].data));
    // new windows get IDs past every ID the part had (the stencil windows replaced too)
    let mut next = parsed.as_ref().and_then(|w| w.children.iter().filter_map(|c| c.id).max()).map_or(0, |m| m.saturating_add(1));
    let (root, kept) = match parsed {
        Some(w) => (w.root, w.children.into_iter().filter(|c| !c.kind.eq_ignore_ascii_case("Stencil")).collect::<Vec<Win>>()),
        None => (
            alloc::vec![
                (String::from("ClientWidth"), String::from("1600")),
                (String::from("ClientHeight"), String::from("900")),
                (String::from("xmlns"), String::from(NS_VISIO)),
                (String::from("xmlns:r"), String::from(NS_REL)),
                (String::from("xml:space"), String::from("preserve")),
            ],
            Vec::new(),
        ),
    };
    let drawing = kept.iter().find(|c| c.kind.eq_ignore_ascii_case("Drawing")).and_then(|c| c.id);
    let mut s = String::from(DECL);
    s.push_str("<Windows");
    for (q, v) in &root {
        attr(&mut s, q, v);
    }
    s.push('>');
    let parent = match drawing {
        Some(id) => id,
        None => {
            let id = next;
            next = next.saturating_add(1);
            let page = doc.settings.top_page.or_else(|| doc.foreground_pages().next().and_then(|p| doc.page(p)).map(|p| p.header.id)).unwrap_or(0);
            s.push_str(&format!("<Window ID='{id}' WindowType='Drawing' WindowState='{WS_MAXIMIZED}' ContainerType='Page' Page='{page}'/>"));
            id
        }
    };
    for w in &kept {
        s.push_str(&w.raw);
    }
    for (k, w) in list.iter().enumerate() {
        let state = if w.active { WS_DOCKED | WS_ACTIVE } else { WS_DOCKED };
        s.push_str(&format!("<Window ID='{next}' WindowType='Stencil' WindowState='{state}' ParentWindow='{parent}'"));
        attr(&mut s, "Document", &w.document);
        s.push_str(&format!("><StencilGroup>10</StencilGroup><StencilGroupPos>{}</StencilGroupPos></Window>", k + 1));
        next = next.saturating_add(1);
    }
    s.push_str("</Windows>");
    let data = s.into_bytes();
    match at {
        Some(i) => doc.parts[i].data = data,
        None => {
            let mut name = String::from("/visio/windows.xml");
            let mut n = 1;
            while doc.parts.iter().any(|p| doc.str(p.name).eq_ignore_ascii_case(&name)) && n < 10_000 {
                name = format!("/visio/windows{n}.xml");
                n += 1;
            }
            let part = Part { name: doc.intern(&name), content_type: doc.intern(CT_WINDOWS), rel_type: doc.intern(REL_WINDOWS), data };
            doc.parts.push(part);
        }
    }
}

// ---------------------------------------------------------------- reading a template

// the value of User.<name>.Value of a sheet as text (no inheritance: these cells are local)
fn user_value(doc: &Document, sheet: SheetRef, name: &str) -> Option<String> {
    let s = doc.sheet(sheet)?;
    let (sec, row, value) = (doc.sym("User")?, doc.sym(name)?, doc.sym("Value")?);
    let r = s.section(SecKey::one(sec))?.row(RowKey::Named(row))?;
    if r.del {
        return None;
    }
    match r.cell(value)?.value {
        Value::Text(t) => Some(String::from(doc.str(t))),
        Value::Num(n) => Some(format!("{n}")),
        Value::Absent => None,
    }
}

// the User rows `prefix` + a number, in sheet order -> (row name, number)
fn numbered(doc: &Document, sheet: SheetRef, prefix: &str) -> Vec<(String, u32)> {
    let (Some(s), Some(sec)) = (doc.sheet(sheet), doc.sym("User")) else { return Vec::new() };
    let Some(section) = s.section(SecKey::one(sec)) else { return Vec::new() };
    section
        .rows
        .iter()
        .filter_map(|r| match r.key {
            RowKey::Named(n) if !r.del => {
                let name = doc.str(n);
                let k = name.strip_prefix(prefix).filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))?.parse::<u32>().ok()?;
                Some((String::from(name), k))
            }
            _ => None,
        })
        .collect()
}

fn drop_user(tx: &mut Transaction, sheet: SheetRef, name: &str) {
    let (Some(u), Some(n)) = (tx.doc().sym("User"), tx.doc().sym(name)) else { return };
    let (sk, rk) = (SecKey::one(u), RowKey::Named(n));
    if tx.doc().sheet(sheet).and_then(|s| s.section(sk)).and_then(|s| s.row(rk)).is_some_and(|r| !r.del) {
        let _ = tx.delete_row(sheet, sk, rk);
    }
}

fn five_cell(doc: &Document, name: &str) -> Option<[String; 5]> {
    user_value(doc, SheetRef::Document, name).map(|t| five_of(&t))
}

/// the catalogue's cells of a template (fields of cells it lacks stay empty)
pub fn info(doc: &Document) -> Info {
    let ds = SheetRef::Document;
    let order = user_value(doc, ds, JFD_ORDER).and_then(|v| v.trim().parse::<f64>().ok()).filter(|x| x.is_finite() && *x >= 0.0).map_or(0, |x| x.min(u32::MAX as f64) as u32);
    Info {
        template: user_value(doc, ds, JFD_TEMPLATE).unwrap_or_default(),
        name: five_cell(doc, instance::JFD_NAME),
        description: five_cell(doc, JFD_DESCRIPTION),
        category: user_value(doc, ds, instance::JFD_CATEGORY).unwrap_or_default(),
        order,
        starter: five_cell(doc, JFD_STARTER),
        default: user_value(doc, ds, JFD_DEFAULT).is_some_and(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("TRUE")),
        registers: registers(doc),
    }
}

/// the context registers a template or a drawing made from one brought (JFD_Registers)
pub fn registers(doc: &Document) -> Vec<String> {
    user_value(doc, SheetRef::Document, JFD_REGISTERS)
        .map(|v| v.split(';').map(str::trim).filter(|s| !s.is_empty()).take(MAX_LIST).map(String::from).collect())
        .unwrap_or_default()
}

// the text of a five-language list in `lang`, else the first one there
fn pick(t: &[String; 5], lang: usize) -> String {
    let own = t.get(lang).map_or("", String::as_str);
    if !own.is_empty() {
        return String::from(own);
    }
    t.iter().find(|s| !s.is_empty()).cloned().unwrap_or_default()
}

// "Section.Row.Cell" (Row a name, or a number for indexed rows) or "Cell" -> its address
fn cell_addr(tx: &mut Transaction, text: &str) -> Option<CellAddr> {
    let parts: Vec<&str> = text.split('.').collect();
    match parts.as_slice() {
        [c] if !c.is_empty() => Some(tx.single(c)),
        [sec, row, c] if !sec.is_empty() && !row.is_empty() && !c.is_empty() => {
            let s = tx.intern(sec);
            let c = tx.intern(c);
            let key = match row.parse::<u32>() {
                Ok(ix) => RowKey::Index(ix),
                Err(_) => RowKey::Named(tx.intern(row)),
            };
            Some(CellAddr::row(SecKey::one(s), key, c))
        }
        _ => None,
    }
}

// one shape of a new drawing in language `lang` -> whether anything changed
fn localize_shape(tx: &mut Transaction, s: ShapeRef, lang: usize) -> bool {
    let sheet = SheetRef::Shape(s);
    let mut changed = false;
    let own = user_value(tx.doc(), sheet, instance::JFD_TEXT);
    if let Some(t) = &own {
        let v = pick(&five_of(t), lang);
        if !v.is_empty() {
            let _ = tx.set_text(s, Some(Text::plain(&v)));
        }
        drop_user(tx, sheet, instance::JFD_TEXT);
        changed = true;
    }
    let Some(m) = tx.doc().shape(s).and_then(|x| x.header().master).and_then(|id| tx.doc().master_by_id(id)) else { return changed };
    let texts = MasterTexts::of(tx.doc(), m);
    // (JD26) the texts of a group master's parts
    if instance::localize_parts(tx, s, lang).is_ok_and(|v| !v.is_empty()) {
        changed = true;
    }
    // the name: the master's in this language, with the ".ID" a second instance got
    let name_u = tx.doc().shape(s).and_then(|x| x.header().name_u).map(|n| String::from(tx.doc().str(n)));
    if let Some(nu) = name_u.filter(|_| !texts.name_u.is_empty()) {
        if let Some(rest) = nu.strip_prefix(texts.name_u.as_str()) {
            let id_suffix = rest.strip_prefix('.').is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
            if rest.is_empty() || id_suffix {
                let name = format!("{}{rest}", texts.name(lang));
                if tx.doc().shape(s).map(|x| x.name()).as_deref() != Some(name.as_str()) && tx.set_shape_names(s, Some(&name), Some(&nu)).is_ok() {
                    changed = true;
                }
            }
        }
    }
    // the master's text where the shape shows one of the master's texts
    if own.is_none() {
        if let Some(mt) = &texts.text {
            let cur = tx.doc().shape(s).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default();
            let cur = cur.trim_end_matches('\n');
            let want = pick(mt, lang);
            if !want.is_empty() && cur != want && mt.iter().any(|x| !x.is_empty() && x == cur) && tx.set_text(s, Some(Text::plain(&want))).is_ok() {
                changed = true;
            }
        }
    }
    // translated cells (Control.Row_1.Prompt ...) where the shape shows one of their texts
    for (c, values) in &texts.translated {
        let want = pick(values, lang);
        if want.is_empty() {
            continue;
        }
        let Some(addr) = cell_addr(tx, c) else { continue };
        let cur = tx.doc().resolve(sheet, &addr).value().text().map(|t| String::from(tx.doc().str(t)));
        let Some(cur) = cur else { continue };
        if cur == want || !values.iter().any(|v| *v == cur) {
            continue;
        }
        let v = tx.intern(&want);
        if tx.set_constant(sheet, &addr, Value::Text(v), Some(Unit::Str)).is_ok() {
            changed = true;
        }
    }
    changed
}

// the rule set's part in language `lang` from the JFD_RuleSetN / JFD_RuleN cells
fn localize_rules(doc: &mut Document, lang: usize) {
    let ds = SheetRef::Document;
    let sets: Vec<Vec<String>> = numbered(doc, ds, JFD_RULE_SET).iter().filter_map(|(n, _)| user_value(doc, ds, n)).map(|t| split_fields(&t)).collect();
    let rules: Vec<Vec<String>> = numbered(doc, ds, JFD_RULE).iter().filter_map(|(n, _)| user_value(doc, ds, n)).map(|t| split_fields(&t)).collect();
    if sets.is_empty() && rules.is_empty() {
        return;
    }
    let Some(i) = doc.parts.iter().position(|p| doc.str(p.content_type) == CT_VALIDATION) else { return };
    let field = |v: &[String], k: usize| -> Option<String> { v.get(k).filter(|s| !s.is_empty()).cloned() };
    let Some(text) = xml::text_of(&doc.parts[i].data).map(|t| t.into_owned()) else { return };
    let mut r = Reader::new(&text, 64, 256);
    let mut out = String::from(DECL);
    // the open elements (None: an empty one, its End writes nothing)
    let mut open: Vec<Option<String>> = Vec::new();
    let mut set = String::new();
    loop {
        let Ok(ev) = r.next() else { return };
        match ev {
            Event::Start(s) => {
                let nu = s.attrs.iter().find(|a| a.local == "NameU" && a.ns == xml::Ns::None).map(|a| a.value.to_string()).unwrap_or_default();
                // the attributes this element gets in `lang`
                let mut repl: Vec<(&str, String)> = Vec::new();
                if s.is("RuleSet") {
                    set = nu.clone();
                    if let Some(v) = sets.iter().find(|v| v.first() == Some(&nu)) {
                        repl.extend(field(v, 1 + lang).map(|x| ("Name", x)));
                        repl.extend(field(v, 6 + lang).map(|x| ("Description", x)));
                    }
                } else if s.is("Rule") {
                    let key = format!("{set}.{nu}");
                    if let Some(v) = rules.iter().find(|v| v.first() == Some(&key)) {
                        repl.extend(field(v, 1 + lang).map(|x| ("Description", x)));
                        repl.extend(field(v, 6 + lang).map(|x| ("Category", x)));
                    }
                }
                out.push('<');
                out.push_str(s.qname);
                for a in &s.attrs {
                    let v = repl.iter().find(|(n, _)| a.ns == xml::Ns::None && a.local == *n).map_or(a.value.as_ref(), |(_, v)| v.as_str());
                    attr(&mut out, a.qname, v);
                }
                if s.empty {
                    out.push_str("/>");
                    open.push(None);
                } else {
                    out.push('>');
                    open.push(Some(String::from(s.qname)));
                }
            }
            Event::End { .. } => {
                if let Some(Some(q)) = open.pop() {
                    out.push_str("</");
                    out.push_str(&q);
                    out.push('>');
                }
            }
            Event::Text(t) => {
                if !open.is_empty() {
                    escape(&mut out, &t, false);
                }
            }
            Event::Eof => break,
        }
    }
    doc.parts[i].data = out.into_bytes();
}

/// make a template a new drawing in language `lang` (an index into instance::LANGS; see the
/// module comment)
pub fn to_drawing(doc: &mut Document, lang: usize) {
    let lang = lang.min(LANGS.len() - 1);
    doc.kind = DocKind::Drawing;
    let mut eng = Engine::new();
    let mut touched: Vec<SheetRef> = Vec::new();
    {
        let mut tx = doc.transaction("Neue Zeichnung");
        let pages: Vec<PageRef> = tx.doc().pages().collect();
        for p in pages {
            let ps = SheetRef::Page(p);
            if let Some(t) = user_value(tx.doc(), ps, instance::JFD_NAME) {
                let name = pick(&five_of(&t), lang);
                let nu = tx.doc().page(p).and_then(|x| x.header.name_u.or(x.header.name)).map(|n| String::from(tx.doc().str(n)));
                if let (false, Some(nu)) = (name.is_empty(), nu) {
                    let _ = tx.rename_page(p, &name, &nu);
                }
                drop_user(&mut tx, ps, instance::JFD_NAME);
            }
            for (row, n) in numbered(tx.doc(), ps, JFD_LAYER) {
                if let Some(t) = user_value(tx.doc(), ps, &row) {
                    let name = pick(&five_of(&t), lang);
                    let (layer, cell_name) = (tx.intern("Layer"), tx.intern("Name"));
                    let a = CellAddr::row(SecKey::one(layer), RowKey::Index(n.saturating_sub(1)), cell_name);
                    if !name.is_empty() && tx.doc().resolve(ps, &a).exists() {
                        let v = tx.intern(&name);
                        let _ = tx.force_constant(ps, &a, Value::Text(v), None);
                    }
                }
                drop_user(&mut tx, ps, &row);
            }
            let shapes: Vec<ShapeRef> = tx.doc().all_shapes(Owner::Page(p)).collect();
            for s in shapes {
                if localize_shape(&mut tx, s, lang) {
                    touched.push(SheetRef::Shape(s));
                }
            }
        }
        for name in [instance::JFD_NAME, JFD_DESCRIPTION, instance::JFD_CATEGORY, JFD_ORDER, JFD_STARTER, JFD_DEFAULT] {
            drop_user(&mut tx, SheetRef::Document, name);
        }
        if !touched.is_empty() {
            let _ = eng.recalc(&mut tx, &touched);
        }
        let _ = tx.commit();
    }
    localize_rules(doc, lang);
}

#[cfg(test)]
mod tests;
