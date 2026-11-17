// theme -- THEMEVAL and its relatives over the document's themes (FX-08, TH-01)
//
// A document keeps its theme parts as a pool (Document::themes); a sheet picks five parts of
// them by number ([MS-VSDX] 2.2.7.4): ColorSchemeIndex the colour scheme, FontSchemeIndex the
// font scheme, EffectSchemeIndex the format scheme (fill, line and effect style lists),
// ConnectorSchemeIndex the connector scheme, ThemeIndex the theme as a whole -- each the
// component whose vt:schemeID schemeEnum has that number, in any part.  65534 means "the
// page's value", 0 no theme (also when any of the five is 0; a master has no page, so its
// shapes are unthemed).
//
// Within the components the shape's quick style cells choose: QuickStyleLineColor /
// FillColor / ShadowColor / FontColor a colour (0 dk1, 1 lt1, 2..7 accent1..6, 8 background,
// 100..106 the variant colours of VariationColorIndex), QuickStyleLineMatrix / FillMatrix /
// EffectsMatrix / FontMatrix an entry of the style lists (1..6; 100..103 the variant style of
// VariationStyleIndex; 0 the root style sheet).  A DrawingML style colours its placeholder
// (phClr) with the chosen colour, through the modifiers it carries (lumMod, satMod, tint ...).
//
//   THEMEVAL()               the theme's value for the cell it stands in: V stores "Themed"
//   THEMEVAL(name [,dflt])   a named value (colours, line, fill, shadow, font properties);
//                            unthemed: dflt, else the "No Theme" colour or the root style's
//                            cell; numbers name quick style colours (THEMEVAL(QuickStyleFillColor))
//   THEMEPROP("MultiFormat" | "Embellishment"), ISTHEMED(), CELLISTHEMED(b), THEMERESTORE(x)
//
// Not known from the documentation and therefore estimated: FillColor2 of a solid fill (the
// fill colour itself), the font scheme used for text (the minor one), cap and compound line
// codes.  JD11 applies themes; this module only reads them.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::color::{hls_to_rgb_f, rgb_to_hls_f, Color};
use super::engine::cell_value;
use super::eval::{page_of, Ev, R};
use super::math;
use super::parse::{Func, Node};
use super::val::{Axis, Val};
use super::xml::Tree;
use crate::cell::CellError;
use crate::document::{Document, SheetRef};
use crate::names::Category;
use crate::sheet::{CellAddr, RowKey, SecKey};

type Rgb = (u8, u8, u8);

/// a theme part read for formulas: its element tree and the components it carries
#[derive(Clone, Debug, Default)]
pub struct Theme {
    /// the element tree
    pub tree: Tree,
    clr: Option<u32>,
    clr_id: Option<u32>,
    font: Option<u32>,
    font_id: Option<u32>,
    fmt: Option<u32>,
    fmt_id: Option<u32>,
    conn: Option<u32>,
    conn_id: Option<u32>,
    theme_id: Option<u32>,
    ext: Option<u32>,
}

fn scheme_id(t: &Tree, i: Option<u32>) -> Option<u32> {
    let s = t.find(i?, "schemeID")?;
    t.attr(s, "schemeEnum")?.parse().ok()
}

impl Theme {
    /// read a theme part
    pub fn parse(data: &[u8]) -> Theme {
        let tree = Tree::parse(data);
        let mut th = Theme::default();
        if let Some(te) = tree.path(0, &["themeElements"]) {
            th.clr = tree.child(te, "clrScheme");
            th.clr_id = th.clr.and_then(|c| scheme_id(&tree, tree.child(c, "extLst")));
            th.font = tree.child(te, "fontScheme");
            th.font_id = th.font.and_then(|c| scheme_id(&tree, tree.child(c, "extLst")));
            th.fmt = tree.child(te, "fmtScheme");
            th.ext = tree.child(te, "extLst");
            if let Some(ext) = th.ext {
                th.fmt_id = scheme_id(&tree, tree.find(ext, "fmtSchemeEx"));
                th.conn = tree.find(ext, "fmtConnectorScheme");
                th.conn_id = scheme_id(&tree, tree.find(ext, "fmtConnectorSchemeEx"));
                th.theme_id = scheme_id(&tree, tree.find(ext, "themeScheme"));
            }
        }
        th.tree = tree;
        th
    }

    fn ext_list(&self, name: &str) -> Vec<u32> {
        let t = &self.tree;
        match self.ext.and_then(|e| t.find(e, name)) {
            Some(l) => t.get(l).map(|e| e.children.clone()).unwrap_or_default(),
            None => Vec::new(),
        }
    }
}

/// the parsed themes of a document (by index into Document::themes), and the theme context
/// of each sheet during one calculation (with the cells it read, for the dependencies)
#[derive(Clone, Default)]
pub(crate) struct Themes {
    parsed: Vec<Option<Rc<Theme>>>,
    done: bool,
    sheets: alloc::collections::BTreeMap<SheetRef, (Rc<Ctx>, Vec<super::engine::NodeKey>)>,
}

impl core::fmt::Debug for Themes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Themes({} parts)", self.parsed.len())
    }
}

impl Themes {
    /// forget the sheets' contexts (a new calculation)
    pub(crate) fn new_run(&mut self) {
        self.sheets.clear();
    }
}

impl Themes {
    fn all(&mut self, doc: &Document) -> Vec<Rc<Theme>> {
        if !self.done {
            self.parsed = doc.themes.iter().map(|t| Some(Rc::new(Theme::parse(&t.data)))).collect();
            self.done = true;
        }
        self.parsed.iter().flatten().cloned().collect()
    }
}

// ---- the theme of a sheet

const NO_THEME: [Rgb; 9] = [
    (0, 0, 0), (255, 255, 255), (0xc0, 0x50, 0x46), (0x9d, 0xbb, 0x61), (0xab, 0x9a, 0xc0), (0x4b, 0xac, 0xc6),
    (0xf5, 0x9d, 0x56), (0xff, 0xc0, 0x00), (255, 255, 255),
];

struct Ctx {
    themed: bool,
    clr: Option<(Rc<Theme>, u32)>,
    font: Option<(Rc<Theme>, u32)>,
    fmt: Option<(Rc<Theme>, u32)>,
    conn: Option<(Rc<Theme>, u32)>,
    var_color: u32,
    var_style: u32,
    embellishment: u32,
    q: [u32; 10],
    connector: bool,
}

// quick style cells in the order of Ctx::q
const QUICK: [&str; 10] = [
    "QuickStyleLineColor", "QuickStyleFillColor", "QuickStyleShadowColor", "QuickStyleFontColor", "QuickStyleLineMatrix",
    "QuickStyleFillMatrix", "QuickStyleEffectsMatrix", "QuickStyleFontMatrix", "QuickStyleType", "QuickStyleVariation",
];

fn read_num(ev: &mut Ev, sheet: SheetRef, name: &str) -> Option<f64> {
    let c = ev.doc.sym(name)?;
    match cell_value(ev.doc, ev.run, sheet, &CellAddr::single(c)) {
        Val::Err(_) => None,
        v => v.num(Axis::X).ok(),
    }
}

// a theme index cell: 65534 is the page's value
fn index_cell(ev: &mut Ev, name: &str) -> u32 {
    let v = read_num(ev, ev.sheet, name).unwrap_or(0.0);
    let v = if v == 65534.0 {
        match page_of(ev.sheet) {
            Some(p) if p != ev.sheet => read_num(ev, p, name).unwrap_or(0.0),
            _ => 0.0,
        }
    } else {
        v
    };
    if v == 65534.0 { 0 } else { v.clamp(0.0, 65535.0) as u32 }
}

fn pick(parts: &[Rc<Theme>], id: u32, get: fn(&Theme) -> (Option<u32>, Option<u32>)) -> Option<(Rc<Theme>, u32)> {
    if id == 0 {
        return None;
    }
    parts.iter().find_map(|p| match get(p) {
        (Some(el), Some(i)) if i == id => Some((p.clone(), el)),
        _ => None,
    })
}

// whether the sheet's line style chain has the style "Connector"
fn is_connector(doc: &Document, sheet: SheetRef) -> bool {
    let mut cur = Some(sheet);
    let mut n = 0;
    while let (Some(s), true) = (cur, n < 64) {
        if let SheetRef::Style(st) = s {
            if doc.style(st).and_then(|x| x.header.name_u).is_some_and(|nm| doc.str(nm) == "Connector") {
                return true;
            }
        }
        cur = doc.parent_sheet(s, Some(Category::Line));
        n += 1;
    }
    false
}

// the theme context of the evaluated sheet, once per sheet and calculation; a cached one
// still records the cells it was made from
fn ctx(ev: &mut Ev) -> Rc<Ctx> {
    if let Some((c, keys)) = ev.run.themes.sheets.get(&ev.sheet) {
        let (c, keys) = (c.clone(), keys.clone());
        for k in keys {
            ev.run.record_key(k);
        }
        return c;
    }
    let mark = ev.run.reads_len();
    let c = Rc::new(make_ctx(ev));
    let keys = ev.run.reads_since(mark);
    ev.run.themes.sheets.insert(ev.sheet, (c.clone(), keys));
    c
}

fn make_ctx(ev: &mut Ev) -> Ctx {
    let ids = [
        index_cell(ev, "ThemeIndex"),
        index_cell(ev, "ColorSchemeIndex"),
        index_cell(ev, "EffectSchemeIndex"),
        index_cell(ev, "ConnectorSchemeIndex"),
        index_cell(ev, "FontSchemeIndex"),
    ];
    let var_color = index_cell(ev, "VariationColorIndex");
    let var_style = index_cell(ev, "VariationStyleIndex");
    let embellishment = index_cell(ev, "EmbellishmentIndex");
    let mut q = [0u32; 10];
    for (i, n) in QUICK.iter().enumerate() {
        q[i] = read_num(ev, ev.sheet, n).unwrap_or(0.0).clamp(0.0, 65535.0) as u32;
    }
    let parts = ev.run.themes.all(ev.doc);
    let themed = ids.iter().all(|&i| i != 0) && !parts.is_empty();
    let connector = is_connector(ev.doc, ev.sheet);
    Ctx {
        themed,
        clr: pick(&parts, ids[1], |t| (t.clr, t.clr_id)),
        font: pick(&parts, ids[4], |t| (t.font, t.font_id)),
        fmt: pick(&parts, ids[2], |t| (t.fmt, t.fmt_id)),
        conn: pick(&parts, ids[3], |t| (t.conn, t.conn_id)),
        var_color,
        var_style,
        embellishment,
        q,
        connector,
    }
}

// ---- colours of the scheme

fn hex(s: &str) -> Option<Rgb> {
    match Color::parse(&alloc::format!("#{s}"))? {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

const SCHEME: [&str; 12] = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];

impl Ctx {
    fn scheme_color(&self, name: &str) -> Option<Rgb> {
        let (t, c) = self.clr.as_ref()?;
        let name = match name {
            "tx1" => "dk1",
            "bg1" => "lt1",
            "tx2" => "dk2",
            "bg2" => "lt2",
            n => n,
        };
        let el = t.tree.child(*c, name)?;
        let first = *t.tree.get(el)?.children.first()?;
        color_of(&t.tree, first, None, self)
    }

    fn background(&self) -> Option<Rgb> {
        let (t, c) = self.clr.as_ref()?;
        let b = t.tree.find(*c, "bkgnd")?;
        let first = *t.tree.get(b)?.children.first()?;
        color_of(&t.tree, first, None, self)
    }

    fn variation(&self) -> Option<(Rc<Theme>, u32)> {
        let (t, c) = self.clr.as_ref()?;
        let list = t.tree.find(*c, "variationClrSchemeLst")?;
        let v = t.tree.children(list, "variationClrScheme").nth(self.var_color as usize)?;
        Some((t.clone(), v))
    }

    fn variant_color(&self, k: u32) -> Option<Rgb> {
        let (t, v) = self.variation()?;
        let el = t.tree.child(v, &alloc::format!("varColor{}", k + 1))?;
        let first = *t.tree.get(el)?.children.first()?;
        color_of(&t.tree, first, None, self)
    }

    // a quick style colour number: 0 dk1, 1 lt1, 2..7 accents, 8 background, 100.. variants
    fn slice(&self, n: u32) -> Option<Rgb> {
        if !self.themed {
            return match n {
                0..=8 => Some(NO_THEME[n as usize]),
                100..=106 => Some(NO_THEME[(2 + (n - 100) as usize).min(7)]),
                _ => None,
            };
        }
        match n {
            0 => self.scheme_color("dk1"),
            1 => self.scheme_color("lt1"),
            2..=7 => self.scheme_color(SCHEME[(n + 2) as usize]),
            8 => self.background().or_else(|| self.scheme_color("lt1")),
            100..=106 => self.variant_color(n - 100),
            _ => None,
        }
    }

    // the style list index of a matrix cell: 1..6 as such, 100..103 through the variant style
    fn matrix(&self, which: usize) -> u32 {
        let m = self.q[4 + which];
        if (100..=103).contains(&m) {
            if let Some((t, _)) = self.fmt.as_ref().or(self.clr.as_ref()) {
                let lst = t.ext_list("variationStyleSchemeLst");
                if let Some(&scheme) = lst.get(self.var_style as usize) {
                    if let Some(vs) = t.tree.children(scheme, "varStyle").nth((m - 100) as usize) {
                        let attr = ["lineIdx", "fillIdx", "effectIdx", "fontIdx"][which];
                        return t.tree.attr(vs, attr).and_then(|v| v.parse().ok()).unwrap_or(0);
                    }
                }
            }
            return 0;
        }
        m
    }

    // the format scheme for line, fill and effects (QuickStyleType: 0 by kind, 1-2 effect
    // scheme, 3 connector scheme)
    fn scheme_for(&self) -> Option<&(Rc<Theme>, u32)> {
        let conn = match self.q[8] {
            1 | 2 => false,
            3 => true,
            _ => self.connector,
        };
        if conn { self.conn.as_ref().or(self.fmt.as_ref()) } else { self.fmt.as_ref() }
    }
}

// a DrawingML colour element with its modifiers; phClr is `ph`
fn color_of(t: &Tree, el: u32, ph: Option<Rgb>, cx: &Ctx) -> Option<Rgb> {
    let e = t.get(el)?;
    let base = match e.name.as_str() {
        "srgbClr" => hex(t.attr(el, "val")?)?,
        "sysClr" => hex(t.attr(el, "lastClr").unwrap_or("000000"))?,
        "schemeClr" => match t.attr(el, "val")? {
            "phClr" => ph?,
            v => cx.scheme_color(v)?,
        },
        "prstClr" => match t.attr(el, "val")? {
            "black" => (0, 0, 0),
            "white" => (255, 255, 255),
            _ => return None,
        },
        _ => return None,
    };
    Some(modify(t, el, base))
}

// the alpha of a colour element (1 opaque)
fn alpha_of(t: &Tree, el: u32) -> f64 {
    t.child(el, "alpha").and_then(|a| t.attr(a, "val")).and_then(|v| v.parse::<f64>().ok()).map_or(1.0, |v| v / 100_000.0)
}

// ECMA-376 colour transforms in the order written (tint, shade, lum/sat/hue mod and off)
fn modify(t: &Tree, el: u32, c: Rgb) -> Rgb {
    let mut rgb = (c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0);
    let Some(e) = t.get(el) else { return c };
    for &m in &e.children {
        let Some(me) = t.get(m) else { continue };
        let v = t.attr(m, "val").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
        match me.name.as_str() {
            "tint" => rgb = (rgb.0 * v + (1.0 - v), rgb.1 * v + (1.0 - v), rgb.2 * v + (1.0 - v)),
            "shade" => rgb = (rgb.0 * v, rgb.1 * v, rgb.2 * v),
            n @ ("lumMod" | "lumOff" | "satMod" | "satOff" | "hueMod" | "hueOff") => {
                let q = |x: f64| (math::round(x.clamp(0.0, 1.0) * 255.0)) as u8;
                let (mut h, mut l, mut s) = rgb_to_hls_f((q(rgb.0), q(rgb.1), q(rgb.2)));
                match n {
                    "lumMod" => l *= v,
                    "lumOff" => l += v,
                    "satMod" => s *= v,
                    "satOff" => s += v,
                    // hue in 60000ths of a degree: the value read is /100000, so * 100000 / 60000 / 360
                    "hueMod" => h *= v,
                    _ => h += v * 100_000.0 / 60_000.0 / 360.0,
                }
                rgb = hls_to_rgb_f(math::modulus(h, 1.0), l.clamp(0.0, 1.0), s.clamp(0.0, 1.0));
            }
            _ => {}
        }
    }
    let q = |x: f64| (math::round(x.clamp(0.0, 1.0) * 255.0)) as u8;
    (q(rgb.0), q(rgb.1), q(rgb.2))
}

// the colour element inside a fill or line element (solidFill / first gradient stop)
fn fill_color_el(t: &Tree, fill: u32, last: bool) -> Option<u32> {
    let f = t.get(fill)?;
    match f.name.as_str() {
        "solidFill" => f.children.first().copied(),
        "gradFill" => {
            let gs = t.child(fill, "gsLst")?;
            let stops: Vec<u32> = t.children(gs, "gs").collect();
            let s = if last { *stops.last()? } else { *stops.first()? };
            t.get(s)?.children.first().copied()
        }
        "ln" => {
            let sf = t.child(fill, "solidFill").or_else(|| t.child(fill, "gradFill"))?;
            fill_color_el(t, sf, last)
        }
        _ => None,
    }
}

// ---- named values

#[derive(Copy, Clone, PartialEq, Debug)]
enum Prop {
    Color(u32),
    TextColor,
    FillColor,
    FillColor2,
    LineColor(bool),
    ShadowColor,
    Font(u8),
    TextStyle,
    Line(&'static str, bool),
    Fill(&'static str),
    Shadow(&'static str),
    Stop(bool, u32, u8),
    Root(&'static str),
}

// the named values ([MS-VSDX] 2.5.9.12 and the ShapeSheet reference; names in any case)
fn prop_of(name: &str) -> Option<Prop> {
    let n = name.to_ascii_lowercase();
    let line = |s: &str| -> Option<&'static str> {
        Some(match s {
            "weight" => "LineWeight",
            "pattern" => "LinePattern",
            "cap" => "LineCap",
            "rounding" => "Rounding",
            "transparency" | "colortrans" => "LineColorTrans",
            "compoundtype" => "CompoundType",
            "begin" => "BeginArrow",
            "end" => "EndArrow",
            "beginsize" => "BeginArrowSize",
            "endsize" => "EndArrowSize",
            _ => return None,
        })
    };
    if let Some(rest) = n.strip_prefix("accentcolor") {
        return match rest {
            "" => Some(Prop::Color(2)),
            d => d.parse::<u32>().ok().filter(|k| (2..=6).contains(k)).map(|k| Prop::Color(k + 1)),
        };
    }
    if let Some(d) = n.strip_prefix("variantcolor") {
        return d.parse::<u32>().ok().filter(|k| (1..=7).contains(k)).map(|k| Prop::Color(99 + k));
    }
    for (pre, line_kind) in [("linestop", true), ("fillstop", false)] {
        if let Some(rest) = n.strip_prefix(pre) {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let k: u32 = digits.parse().ok()?;
            let what = match &rest[digits.len()..] {
                "color" => 0,
                "transparency" => 1,
                "position" => 2,
                _ => return None,
            };
            return (1..=10).contains(&k).then_some(Prop::Stop(line_kind, k - 1, what));
        }
    }
    if let Some(rest) = n.strip_prefix("connector") {
        return match rest {
            "color" => Some(Prop::LineColor(true)),
            r => line(r).map(|c| Prop::Line(c, true)),
        };
    }
    if let Some(rest) = n.strip_prefix("line") {
        match rest {
            "color" => return Some(Prop::LineColor(false)),
            "rounding" => return Some(Prop::Line("Rounding", false)),
            r => {
                if let Some(c) = line(r) {
                    return Some(Prop::Line(c, false));
                }
            }
        }
    }
    Some(match n.as_str() {
        "dark" => Prop::Color(0),
        "light" => Prop::Color(1),
        "backgroundcolor" => Prop::Color(8),
        "textcolor" | "charactercolor" => Prop::TextColor,
        "fillcolor" => Prop::FillColor,
        "fillcolor2" => Prop::FillColor2,
        "shadowcolor" => Prop::ShadowColor,
        "latinfont" => Prop::Font(0),
        "asianfont" => Prop::Font(1),
        "complexfont" => Prop::Font(2),
        "textstyle" => Prop::TextStyle,
        "filltransparency" => Prop::Fill("FillForegndTrans"),
        "filltransparencybkgnd" => Prop::Fill("FillBkgndTrans"),
        "fillpattern" => Prop::Fill("FillPattern"),
        "shadowtransparency" => Prop::Shadow("ShdwForegndTrans"),
        "shadowpattern" => Prop::Shadow("ShdwPattern"),
        "shadowstyle" | "shadowtype" => Prop::Shadow("ShapeShdwType"),
        "shadowxoffset" => Prop::Shadow("ShapeShdwOffsetX"),
        "shadowyoffset" => Prop::Shadow("ShapeShdwOffsetY"),
        "shadowmagnification" => Prop::Shadow("ShapeShdwScaleFactor"),
        "shadowdirection" => Prop::Shadow("ShapeShdwObliqueAngle"),
        "shadowblur" => Prop::Shadow("ShapeShdwBlur"),
        "linegradientenabled" => Prop::Root("LineGradientEnabled"),
        "linegradientdir" => Prop::Root("LineGradientDir"),
        "linegradientangle" => Prop::Root("LineGradientAngle"),
        "fillgradientenabled" => Prop::Fill("FillGradientEnabled"),
        "fillgradientdir" => Prop::Root("FillGradientDir"),
        "fillgradientangle" => Prop::Root("FillGradientAngle"),
        "rotategradientwithshape" => Prop::Root("RotateGradientWithShape"),
        "usegroupgradient" => Prop::Root("UseGroupGradient"),
        "beveltoptype" => Prop::Root("BevelTopType"),
        "beveltopwidth" => Prop::Root("BevelTopWidth"),
        "beveltopheight" => Prop::Root("BevelTopHeight"),
        "bevelcontourcolor" => Prop::Root("BevelContourColor"),
        "bevelcontoursize" => Prop::Root("BevelContourSize"),
        "bevelmaterial" => Prop::Root("BevelMaterialType"),
        "bevellightingtype" | "bevellighting" => Prop::Root("BevelLightingType"),
        "bevellightingangle" => Prop::Root("BevelLightingAngle"),
        "glowsize" => Prop::Root("GlowSize"),
        "glowcolor" => Prop::Root("GlowColor"),
        "glowcolortrans" | "glowtransparency" => Prop::Root("GlowColorTrans"),
        "reflectionblur" => Prop::Root("ReflectionBlur"),
        "reflectiondist" => Prop::Root("ReflectionDist"),
        "reflectionsize" => Prop::Root("ReflectionSize"),
        "reflectiontrans" => Prop::Root("ReflectionTrans"),
        "softedgessize" => Prop::Root("SoftEdgesSize"),
        "sketchamount" => Prop::Root("SketchAmount"),
        "sketchenabled" => Prop::Root("SketchEnabled"),
        "sketchfillchange" => Prop::Root("SketchFillChange"),
        "sketchlinechange" => Prop::Root("SketchLineChange"),
        "sketchlineweight" => Prop::Root("SketchLineWeight"),
        _ => return None,
    })
}

// the value THEMEVAL() stands for in a cell
fn host_prop(doc: &Document, cell: &str, addr: &CellAddr, connector: bool) -> Option<Prop> {
    if let Some(sk) = addr.section {
        let sec = doc.str(sk.name);
        let ix = match addr.row {
            Some(RowKey::Index(i)) => i,
            _ => 0,
        };
        return match (sec, cell) {
            ("Character", "Color") => Some(Prop::TextColor),
            ("Character", "Font") => Some(Prop::Font(0)),
            ("Character", "AsianFont") => Some(Prop::Font(1)),
            ("Character", "ComplexScriptFont") => Some(Prop::Font(2)),
            ("Character", "Style") => Some(Prop::TextStyle),
            ("FillGradient" | "LineGradient", c) => {
                let what = match c {
                    "GradientStopColor" => 0,
                    "GradientStopColorTrans" => 1,
                    "GradientStopPosition" => 2,
                    _ => return None,
                };
                Some(Prop::Stop(sec == "LineGradient", ix, what))
            }
            _ => None,
        };
    }
    Some(match cell {
        "LineColor" => Prop::LineColor(connector),
        "LineWeight" | "LinePattern" | "LineCap" | "Rounding" | "LineColorTrans" | "CompoundType" | "BeginArrow" | "EndArrow"
        | "BeginArrowSize" | "EndArrowSize" => Prop::Line(static_name(cell)?, connector),
        "FillForegnd" => Prop::FillColor,
        "FillBkgnd" => Prop::FillColor2,
        "FillForegndTrans" | "FillBkgndTrans" | "FillPattern" | "FillGradientEnabled" => Prop::Fill(static_name(cell)?),
        "ShdwForegnd" => Prop::ShadowColor,
        "ShdwForegndTrans" | "ShdwPattern" | "ShapeShdwType" | "ShapeShdwOffsetX" | "ShapeShdwOffsetY" | "ShapeShdwScaleFactor"
        | "ShapeShdwObliqueAngle" | "ShapeShdwBlur" => Prop::Shadow(static_name(cell)?),
        c => Prop::Root(static_name(c)?),
    })
}

// the cell names the module refers to, as 'static
fn static_name(c: &str) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "LineWeight", "LinePattern", "LineCap", "Rounding", "LineColorTrans", "CompoundType", "BeginArrow", "EndArrow", "BeginArrowSize",
        "EndArrowSize", "FillForegndTrans", "FillBkgndTrans", "FillPattern", "FillGradientEnabled", "ShdwForegndTrans", "ShdwPattern",
        "ShapeShdwType", "ShapeShdwOffsetX", "ShapeShdwOffsetY", "ShapeShdwScaleFactor", "ShapeShdwObliqueAngle", "ShapeShdwBlur",
        "LineGradientEnabled", "LineGradientDir", "LineGradientAngle", "FillGradientDir", "FillGradientAngle", "RotateGradientWithShape",
        "UseGroupGradient", "BevelTopType", "BevelTopWidth", "BevelTopHeight", "BevelBottomType", "BevelBottomWidth", "BevelBottomHeight",
        "BevelDepthColor", "BevelDepthSize", "BevelContourColor", "BevelContourSize", "BevelMaterialType", "BevelLightingType",
        "BevelLightingAngle", "GlowSize", "GlowColor", "GlowColorTrans", "ReflectionBlur", "ReflectionDist", "ReflectionSize",
        "ReflectionTrans", "SoftEdgesSize", "SketchAmount", "SketchEnabled", "SketchFillChange", "SketchLineChange", "SketchLineWeight",
        "SketchSeed", "ShapeShdwShow", "LineColor", "FillForegnd", "FillBkgnd", "ShdwForegnd", "TextBkgnd", "TextBkgndTrans",
    ];
    NAMES.iter().find(|n| **n == c).copied()
}

// the root style sheet's value of a cell (the "No Style" style, ID 0)
fn root(ev: &mut Ev, addr: CellAddr) -> Val {
    match ev.doc.style_by_id(0) {
        Some(st) => cell_value(ev.doc, ev.run, SheetRef::Style(st), &addr),
        None => Val::Num(0.0),
    }
}

fn root_single(ev: &mut Ev, name: &str) -> Val {
    match ev.doc.sym(name) {
        Some(c) => root(ev, CellAddr::single(c)),
        None => Val::Num(0.0),
    }
}

fn root_char(ev: &mut Ev, name: &str) -> Val {
    match (ev.doc.sym("Character"), ev.doc.sym(name)) {
        (Some(s), Some(c)) => root(ev, CellAddr::row(SecKey::one(s), RowKey::Index(0), c)),
        _ => Val::Num(0.0),
    }
}

const EMU: f64 = 914_400.0;

fn attr_num(t: &Tree, el: u32, a: &str) -> Option<f64> {
    t.attr(el, a)?.parse().ok()
}

// the luminance fix of QuickStyleVariation ([MS-VSDX] 2.4.4.278): a colour too close to
// the background gives way
fn contrast(ev: &mut Ev, cx: &Ctx, which: u32, c: Rgb) -> Rgb {
    if cx.q[9] & which == 0 {
        return c;
    }
    let Some(bg) = cx.slice(8) else { return c };
    let l = |x: Rgb| super::color::rgb_to_hsl(x.0, x.1, x.2).2;
    if (l(bg) - l(c)).abs() >= 40 {
        return c;
    }
    if l(bg) <= 175 {
        return (255, 255, 255);
    }
    let _ = ev;
    let cands = [cx.slice(cx.q[1]), cx.slice(cx.q[0])];
    cands.into_iter().flatten().max_by_key(|x| (l(*x) - l(bg)).abs()).unwrap_or(c)
}

fn rgb(c: Rgb) -> Val {
    Val::Color(Color::Rgb(c.0, c.1, c.2))
}

// the themed value of a property (None: the theme has none; the caller falls back)
fn themed_value(ev: &mut Ev, cx: &Ctx, p: Prop) -> Option<Val> {
    match p {
        Prop::Color(n) => cx.slice(n).map(rgb),
        Prop::LineColor(conn) => {
            let slice = cx.slice(cx.q[0])?;
            let scheme = if conn { cx.conn.as_ref().or(cx.fmt.as_ref()) } else { cx.scheme_for() };
            let i = cx.matrix(0);
            let c = scheme.and_then(|(t, f)| {
                let l = t.tree.child(*f, "lnStyleLst")?;
                let ln = t.tree.children(l, "ln").nth(i.checked_sub(1)? as usize)?;
                let el = fill_color_el(&t.tree, ln, false)?;
                color_of(&t.tree, el, Some(slice), cx)
            });
            let c = c.unwrap_or(slice);
            Some(rgb(contrast(ev, cx, 4, c)))
        }
        Prop::FillColor | Prop::FillColor2 => {
            let slice = cx.slice(cx.q[1])?;
            let i = cx.matrix(1);
            let c = cx.scheme_for().and_then(|(t, f)| {
                let l = t.tree.child(*f, "fillStyleLst")?;
                let fill = *t.tree.get(l)?.children.get(i.checked_sub(1)? as usize)?;
                let el = fill_color_el(&t.tree, fill, p == Prop::FillColor2)?;
                color_of(&t.tree, el, Some(slice), cx)
            });
            let c = c.unwrap_or(slice);
            Some(rgb(if p == Prop::FillColor { contrast(ev, cx, 8, c) } else { c }))
        }
        Prop::ShadowColor => {
            // the effect style's shadow colour; without a shadow there black (the files)
            let slice = cx.slice(cx.q[2])?;
            let i = cx.matrix(2);
            let c = cx.scheme_for().and_then(|(t, f)| {
                let l = t.tree.child(*f, "effectStyleLst")?;
                let es = t.tree.children(l, "effectStyle").nth(i.checked_sub(1)? as usize)?;
                match t.tree.find(es, "outerShdw") {
                    Some(sh) => {
                        let el = *t.tree.get(sh)?.children.first()?;
                        color_of(&t.tree, el, Some(slice), cx)
                    }
                    None => Some((0, 0, 0)),
                }
            });
            Some(rgb(c.unwrap_or(slice)))
        }
        Prop::TextColor => {
            let slice = cx.slice(cx.q[3])?;
            let i = cx.matrix(3);
            let c = font_props(cx, i).and_then(|(t, fp)| {
                let col = t.tree.child(fp, "color")?;
                let el = *t.tree.get(col)?.children.first()?;
                color_of(&t.tree, el, Some(slice), cx)
            });
            Some(rgb(contrast(ev, cx, 2, c.unwrap_or(slice))))
        }
        Prop::TextStyle => {
            let i = cx.matrix(3);
            let (t, fp) = font_props(cx, i)?;
            attr_num(&t.tree, fp, "style").map(Val::Num)
        }
        Prop::Font(k) => {
            let (t, f) = cx.font.as_ref()?;
            let minor = t.tree.child(*f, "minorFont")?;
            let el = t.tree.child(minor, ["latin", "ea", "cs"][k as usize])?;
            let face = t.tree.attr(el, "typeface")?;
            (!face.is_empty()).then(|| Val::Font(Rc::from(face)))
        }
        Prop::Line(cell, conn) => {
            let i = cx.matrix(0);
            if i == 0 {
                return None;
            }
            let scheme = if conn { cx.conn.as_ref().or(cx.fmt.as_ref()) } else { cx.scheme_for() };
            let (t, f) = scheme?;
            let is_conn = cx.conn.as_ref().is_some_and(|c| Rc::ptr_eq(&c.0, t) && c.1 == *f);
            let l = t.tree.child(*f, "lnStyleLst")?;
            let ln = t.tree.children(l, "ln").nth(i as usize - 1)?;
            let ex = t.ext_list(if is_conn { "fmtConnectorSchemeLineStyles" } else { "fmtSchemeLineStyles" });
            let line_ex = ex.get(i as usize - 1).and_then(|&ls| t.tree.child(ls, "lineEx"));
            let tr = &t.tree;
            let exn = |a: &str| line_ex.and_then(|e| attr_num(tr, e, a));
            match cell {
                "LineWeight" => attr_num(tr, ln, "w").map(|w| Val::Num(w / EMU)),
                // Visio: 0 round, 1 square (ends at the point), 2 extended (half the weight beyond)
                "LineCap" => tr.attr(ln, "cap").map(|c| Val::Num(match c {
                    "rnd" => 0.0,
                    "flat" => 1.0,
                    _ => 2.0,
                })),
                "CompoundType" => tr.attr(ln, "cmpd").map(|c| Val::Num(match c {
                    "dbl" => 1.0,
                    "thickThin" => 2.0,
                    "thinThick" => 3.0,
                    "tri" => 4.0,
                    _ => 0.0,
                })),
                "LineColorTrans" => {
                    let el = fill_color_el(tr, ln, false)?;
                    Some(Val::Num(1.0 - alpha_of(tr, el)))
                }
                "LinePattern" => exn("pattern").map(Val::Num),
                "Rounding" => exn("rndg").map(|r| Val::Num(r / EMU)),
                "BeginArrow" => exn("start").map(Val::Num),
                "EndArrow" => exn("end").map(Val::Num),
                "BeginArrowSize" => exn("startSize").map(Val::Num),
                "EndArrowSize" => exn("endSize").map(Val::Num),
                _ => None,
            }
        }
        Prop::Fill(cell) => {
            let i = cx.matrix(1);
            if i == 0 {
                return None;
            }
            let (t, f) = cx.scheme_for()?;
            let tr = &t.tree;
            let l = tr.child(*f, "fillStyleLst")?;
            let fill = *tr.get(l)?.children.get(i as usize - 1)?;
            match cell {
                "FillPattern" => {
                    let props = t.ext_list("fillStyles");
                    props.get(i as usize - 1).and_then(|&fp| attr_num(tr, fp, "pattern")).map(Val::Num)
                }
                "FillGradientEnabled" => Some(Val::bool(tr.get(fill)?.name == "gradFill")),
                "FillForegndTrans" | "FillBkgndTrans" => {
                    let el = fill_color_el(tr, fill, cell == "FillBkgndTrans")?;
                    Some(Val::Num(1.0 - alpha_of(tr, el)))
                }
                _ => None,
            }
        }
        Prop::Shadow(cell) => {
            let i = cx.matrix(2);
            if i == 0 {
                return None;
            }
            let (t, f) = cx.scheme_for()?;
            let tr = &t.tree;
            let l = tr.child(*f, "effectStyleLst")?;
            let es = tr.children(l, "effectStyle").nth(i as usize - 1)?;
            let sh = tr.find(es, "outerShdw");
            match (cell, sh) {
                // the shadow type of a themed shape is 1 (the files), with or without a shadow
                ("ShapeShdwType", _) => Some(Val::Num(1.0)),
                ("ShdwPattern", None) => Some(Val::Num(0.0)),
                ("ShdwPattern", Some(_)) => Some(Val::Num(1.0)),
                (_, None) => None,
                ("ShapeShdwOffsetX" | "ShapeShdwOffsetY", Some(s)) => {
                    let dist = attr_num(tr, s, "dist").unwrap_or(0.0) / EMU;
                    let dir = attr_num(tr, s, "dir").unwrap_or(0.0) / 60_000.0 * core::f64::consts::PI / 180.0;
                    let (sn, cs) = math::sincos(dir);
                    Some(Val::Num(if cell == "ShapeShdwOffsetX" { dist * cs } else { -dist * sn }))
                }
                ("ShapeShdwBlur", Some(s)) => Some(Val::Num(attr_num(tr, s, "blurRad").unwrap_or(0.0) / EMU)),
                ("ShapeShdwScaleFactor", Some(s)) => Some(Val::Num(attr_num(tr, s, "sx").unwrap_or(100_000.0) / 100_000.0)),
                ("ShdwForegndTrans", Some(s)) => {
                    let el = *tr.get(s)?.children.first()?;
                    Some(Val::Num(1.0 - alpha_of(tr, el)))
                }
                _ => None,
            }
        }
        Prop::Stop(line, k, what) => {
            let (t, f) = cx.scheme_for()?;
            let tr = &t.tree;
            let i = cx.matrix(if line { 0 } else { 1 });
            let style = if line {
                let l = tr.child(*f, "lnStyleLst")?;
                let ln = tr.children(l, "ln").nth(i.checked_sub(1)? as usize)?;
                tr.child(ln, "gradFill")?
            } else {
                let l = tr.child(*f, "fillStyleLst")?;
                *tr.get(l)?.children.get(i.checked_sub(1)? as usize)?
            };
            let gs = tr.child(style, "gsLst")?;
            let stop = tr.children(gs, "gs").nth(k as usize)?;
            match what {
                2 => attr_num(tr, stop, "pos").map(|p| Val::Num(p / 100_000.0)),
                w => {
                    let el = *tr.get(stop)?.children.first()?;
                    let slice = cx.slice(cx.q[if line { 0 } else { 1 }])?;
                    if w == 0 {
                        color_of(tr, el, Some(slice), cx).map(rgb)
                    } else {
                        Some(Val::Num(1.0 - alpha_of(tr, el)))
                    }
                }
            }
        }
        Prop::Root(_) => None,
    }
}

fn font_props(cx: &Ctx, i: u32) -> Option<(Rc<Theme>, u32)> {
    let (t, _) = cx.fmt.as_ref().or(cx.clr.as_ref())?;
    let list = t.ext_list(if cx.connector { "connectorFontStyles" } else { "fontStyles" });
    let fp = *list.get(i.checked_sub(1)? as usize)?;
    Some((t.clone(), fp))
}

// the value without a theme: THEMEVAL() the root style's value of its cell, a name the
// "No Theme" colour or the root style's cell
fn unthemed(ev: &mut Ev, cx: &Ctx, p: Prop, addr: Option<CellAddr>) -> Val {
    if let Some(a) = addr {
        return root(ev, a);
    }
    match p {
        Prop::Color(n) => cx.slice(n).map_or(Val::Num(0.0), rgb),
        Prop::TextColor => rgb((0, 0, 0)),
        Prop::FillColor => rgb((0x96, 0xaf, 0xcf)),
        Prop::FillColor2 => rgb((0xbf, 0xce, 0xe1)),
        Prop::LineColor(_) => rgb((0x1f, 0x47, 0x7d)),
        Prop::ShadowColor => rgb((0x46, 0x46, 0x46)),
        Prop::Font(k) => root_char(ev, ["Font", "AsianFont", "ComplexScriptFont"][k as usize]),
        Prop::TextStyle => root_char(ev, "Style"),
        Prop::Line(c, _) | Prop::Fill(c) | Prop::Shadow(c) | Prop::Root(c) => root_single(ev, c),
        Prop::Stop(..) => Val::Num(0.0),
    }
}

pub(crate) fn call(ev: &mut Ev, f: Func, a: &[u32]) -> R {
    match f {
        Func::ThemeVal => themeval(ev, a),
        Func::ThemeProp => {
            if a.len() != 1 {
                return Err(CellError::Value);
            }
            let name = ev.text(a[0])?.to_ascii_lowercase();
            let cx = ctx(ev);
            if name != "multiformat" && name != "embellishment" {
                return Err(CellError::Value);
            }
            if !cx.themed {
                return Ok(Val::Num(0.0));
            }
            if name == "embellishment" {
                if cx.embellishment != 0 {
                    return Ok(Val::Num(cx.embellishment as f64));
                }
                let v = cx.fmt.as_ref().or(cx.clr.as_ref()).and_then(|(t, _)| {
                    let lst = t.ext_list("variationStyleSchemeLst");
                    attr_num(&t.tree, *lst.get(cx.var_style as usize)?, "embellishment")
                });
                return Ok(Val::Num(v.unwrap_or(0.0)));
            }
            // several colours unless the variant is monotone (the files: no monotone attribute
            // gives MultiFormat true)
            let v = cx.variation().and_then(|(t, v)| attr_num(&t.tree, v, "monotone"));
            Ok(Val::bool(v.unwrap_or(0.0) == 0.0))
        }
        Func::IsThemed => {
            let cx = ctx(ev);
            Ok(Val::bool(cx.themed))
        }
        Func::CellIsThemed => {
            if a.len() != 1 {
                return Err(CellError::Value);
            }
            Ok(Val::bool(ev.truth(a[0])?))
        }
        Func::ThemeRestore => Ok(Val::Num(0.0)),
        _ => Err(CellError::Value),
    }
}

fn themeval(ev: &mut Ev, a: &[u32]) -> R {
    if a.len() > 2 {
        return Err(CellError::Value);
    }
    let cx = ctx(ev);
    if a.is_empty() {
        // the cell's own theme value; V stores "Themed"
        let doc = ev.doc;
        let addr = CellAddr { section: None, row: None, cell: ev.cell };
        let host_addr = super::eval::host_addr(ev);
        let cell = doc.str(ev.cell);
        let Some(p) = host_prop(doc, cell, host_addr.as_ref().unwrap_or(&addr), cx.connector) else {
            return Err(CellError::Value);
        };
        let v = if cx.themed { themed_value(ev, &cx, p) } else { None };
        let v = match v {
            Some(v) => v,
            None => unthemed(ev, &cx, p, host_addr),
        };
        return Ok(Val::Themed(Rc::new(v)));
    }
    // a name or a quick style colour number
    let arg = ev.eval(a[0])?;
    let p = match arg.plain() {
        Val::Str(s) => prop_of(s).ok_or(CellError::Value)?,
        v => Prop::Color(v.num(Axis::X)?.clamp(0.0, 65535.0) as u32),
    };
    let has_default = a.len() == 2 && ev.node(a[1]) != Node::Missing;
    if cx.themed {
        if let Some(v) = themed_value(ev, &cx, p) {
            return Ok(v);
        }
    }
    // a font the theme lacks (typeface "") or a font number as default: the root style's font
    // (the files: THEMEVAL("ComplexFont",4) is "Calibri" with and without a theme; Visio's
    // font numbers belong to the machine)
    if let Prop::Font(_) = p {
        let numeric = has_default && !matches!(ev.eval(a[1]), Ok(Val::Str(_)));
        if cx.themed || numeric || !has_default {
            return Ok(root_char(ev, "Font"));
        }
    }
    if has_default {
        return ev.eval(a[1]);
    }
    Ok(unthemed(ev, &cx, p, None))
}
