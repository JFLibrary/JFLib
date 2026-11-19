// design -- typed designs: a DrawingML theme with Visio's extensions (TH-01, TH-03, TH-04)
//
// A design is what a theme part of a .vsdx holds: the DrawingML theme of ECMA-376 Part 1,
// 20.1.6 (colour scheme, font scheme, format scheme with fill, line, effect and background
// style lists) and Visio's extensions of it ([MS-VSDX] 2.5.x, "dynamic theme"): a connector
// format scheme, Visio's own line, fill and font properties of the styles, four variants
// with seven colours and four styles each, the background colour, and the scheme numbers by
// which a page's ThemeIndex, ColorSchemeIndex, EffectSchemeIndex, ConnectorSchemeIndex and
// FontSchemeIndex cells find the parts again.
//
// Only Visio's identifiers are taken from Visio (the user's decision of 2026-11-18): the
// extension URIs, the namespace http://schemas.microsoft.com/office/visio/2012/theme and the
// element and attribute names, read from the user's own files.  Every value -- colours,
// fonts, widths, line ends, which style a variant uses -- is JFLib's own (catalog.rs).
//
//   Design { name, id, colors, background, major_font, minor_font, shapes, connectors, variants }
//   Design::theme_xml()         -> the theme part's bytes (xml.rs)
//   catalog::all()              -> JFDiagrammer's own designs, NetLab first
//   catalog::by_id(id)
//   apply::*                    -> a design or a variant on a page or the document (apply.rs)
//
// How a shape uses a design (formula/theme.rs evaluates it): its QuickStyle colour cells pick
// a colour (0 dark 1, 1 light 1, 2..7 accent 1..6, 8 the background, 100..106 the colours of
// the page's variant), its QuickStyle matrix cells a style of the lists (1..6, or 100..103:
// the fill, line, effect and font style the page's variant names for its style 1..4).  A
// style's colour placeholder (Clr::ph) is the colour picked.
//
// Scheme numbers: [MS-VSDX] CT_SchemeID calls schemeEnum the one-based index of a built-in
// component, 65535 a custom colour scheme found by GUID.  Visio's own designs use small
// numbers; JFLib's designs use numbers from FIRST_ID on, so that no page of ours names one of
// Visio's designs by mistake.

pub mod apply;
pub mod catalog;
mod xml;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

/// the first scheme number of JFLib's own designs (far above Visio's built-in ones)
pub const FIRST_ID: u32 = 60001;

/// the Visio theme extension namespace
pub const NS_VT: &str = "http://schemas.microsoft.com/office/visio/2012/theme";

/// the extension URIs, as Visio names its extension elements
pub mod uri {
    /// clrScheme: vt:schemeID
    pub const COLOR_ID: &str = "{2703A3B3-D2E1-43D9-8057-6E9D74E0F44A}";
    /// clrScheme: vt:bkgnd
    pub const BACKGROUND: &str = "{093E89EA-6996-430E-BFF9-83A9FAAAAB73}";
    /// clrScheme: vt:variationClrSchemeLst
    pub const VARIANT_COLORS: &str = "{DDD2D869-C2EF-471E-B8FA-914AFA308C9F}";
    /// fontScheme: vt:schemeID
    pub const FONT_ID: &str = "{B3C0788F-89CC-4CDB-A57D-0664CFDA821B}";
    /// themeElements: vt:fmtConnectorScheme
    pub const CONNECTOR_SCHEME: &str = "{1342405F-259F-4C95-8CDF-A9DCE2D418A2}";
    /// themeElements: vt:themeScheme
    pub const THEME_ID: &str = "{D75FF423-9257-4291-A4FE-1B2448832E17}";
    /// themeElements: vt:fmtSchemeEx
    pub const FORMAT_ID: &str = "{27CD58D4-7086-4B73-B2AB-7AEBE2148A8C}";
    /// themeElements: vt:fmtConnectorSchemeEx
    pub const CONNECTOR_ID: &str = "{C6430689-8E98-42EC-ADBF-087148533A3F}";
    /// themeElements: vt:fillStyles
    pub const FILL_STYLES: &str = "{56243398-1771-4C39-BF73-A5702A9C147F}";
    /// themeElements: vt:lineStyles
    pub const LINE_STYLES: &str = "{6CAB99AB-0A78-4BAB-B597-526D62367CB4}";
    /// themeElements: vt:fontStylesGroup
    pub const FONT_STYLES: &str = "{EBE24D50-EC5C-4D6F-A1A3-C5F0A18B936A}";
    /// themeElements: vt:variationStyleSchemeLst
    pub const VARIANT_STYLES: &str = "{494CE47F-D151-47DC-95E8-85652EA8A67E}";
}

/// a colour of the scheme, as DrawingML names it in schemeClr
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// dark 1 (text)
    Dk1,
    /// light 1 (background)
    Lt1,
    /// dark 2
    Dk2,
    /// light 2
    Lt2,
    /// accent 1 .. 6
    Accent(u8),
    /// the placeholder a style is coloured with (the shape's QuickStyle colour)
    Ph,
}

impl Slot {
    /// the schemeClr value
    pub fn name(self) -> &'static str {
        match self {
            Slot::Dk1 => "dk1",
            Slot::Lt1 => "lt1",
            Slot::Dk2 => "dk2",
            Slot::Lt2 => "lt2",
            Slot::Accent(k) => ["accent1", "accent2", "accent3", "accent4", "accent5", "accent6"][(k.clamp(1, 6) - 1) as usize],
            Slot::Ph => "phClr",
        }
    }
}

/// a DrawingML colour transform, in thousandths of a percent (100000 = 100 %)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mod {
    /// luminance times
    LumMod(u32),
    /// luminance plus
    LumOff(u32),
    /// saturation times
    SatMod(u32),
    /// opacity
    Alpha(u32),
}

/// a colour: fixed or of the scheme, with transforms
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Clr {
    /// 0xRRGGBB
    Rgb(u32, Vec<Mod>),
    /// a colour of the scheme or the placeholder
    Scheme(Slot, Vec<Mod>),
}

impl Clr {
    /// the placeholder (the colour the shape picks)
    pub fn ph() -> Clr {
        Clr::Scheme(Slot::Ph, Vec::new())
    }
    /// a scheme colour
    pub fn slot(s: Slot) -> Clr {
        Clr::Scheme(s, Vec::new())
    }
    /// a fixed colour
    pub fn rgb(c: u32) -> Clr {
        Clr::Rgb(c, Vec::new())
    }
    /// with one more transform
    pub fn with(mut self, m: Mod) -> Clr {
        match &mut self {
            Clr::Rgb(_, v) | Clr::Scheme(_, v) => v.push(m),
        }
        self
    }
    /// lighter: luminance times (100 - pct) % plus pct %, toward white
    pub fn lighter(self, pct: u32) -> Clr {
        let p = pct.min(100) * 1000;
        self.with(Mod::LumMod(100_000 - p)).with(Mod::LumOff(p))
    }
    /// darker: luminance times (100 - pct) %
    pub fn darker(self, pct: u32) -> Clr {
        self.with(Mod::LumMod(100_000 - pct.min(100) * 1000))
    }
}

/// a fill style: its colour and Visio's fill pattern (0 no fill, 1 solid)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Fill {
    /// the colour
    pub color: Clr,
    /// vt:fillProps pattern
    pub pattern: u8,
}

impl Fill {
    /// a solid fill
    pub fn solid(color: Clr) -> Fill {
        Fill { color, pattern: 1 }
    }
    /// no fill (Visio's pattern 0; the colour still feeds FillForegnd)
    pub fn none() -> Fill {
        Fill { color: Clr::slot(Slot::Lt1), pattern: 0 }
    }
}

/// a line cap (DrawingML ln@cap)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    /// rnd
    Round,
    /// sq
    Square,
    /// flat
    Flat,
}

/// a line style: DrawingML's ln and Visio's vt:lineEx
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Line {
    /// the width in EMU (12700 a point)
    pub width: u32,
    /// the cap
    pub cap: Cap,
    /// the colour
    pub color: Clr,
    /// Visio's line pattern (0 none, 1 solid, 2 dashed ...)
    pub pattern: u8,
    /// corner rounding in EMU
    pub rounding: u32,
    /// line ends (Visio's arrow numbers, 0 none) and their sizes (0 .. 6)
    pub begin: u8,
    pub begin_size: u8,
    pub end: u8,
    pub end_size: u8,
}

/// EMU per point
pub const EMU_PT: u32 = 12700;

impl Line {
    /// a solid line of `pt` hundredths of a point
    pub fn solid(centi_pt: u32, color: Clr) -> Line {
        Line { width: centi_pt * EMU_PT / 100, cap: Cap::Flat, color, pattern: 1, rounding: 0, begin: 0, begin_size: 2, end: 0, end_size: 2 }
    }
    /// with a line end at the end
    pub fn arrow(mut self, end: u8, size: u8) -> Line {
        self.end = end;
        self.end_size = size;
        self
    }
    /// with rounded corners (EMU)
    pub fn rounded(mut self, emu: u32) -> Line {
        self.rounding = emu;
        self
    }
    /// with a pattern
    pub fn pattern(mut self, p: u8) -> Line {
        self.pattern = p;
        self
    }
}

/// an outer shadow (DrawingML outerShdw)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Shadow {
    /// blur radius and distance in EMU, direction in 60000ths of a degree
    pub blur: u32,
    pub dist: u32,
    pub dir: u32,
    /// the colour (with an alpha transform for its strength)
    pub color: Clr,
}

/// an effect style: nothing or a shadow
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Effect {
    /// the shadow
    pub shadow: Option<Shadow>,
}

/// a font style of Visio's extension: Char.Style bits (1 bold, 2 italic, 4 underline) and
/// the text colour
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontStyle {
    /// Char.Style
    pub style: u8,
    /// the colour
    pub color: Clr,
}

impl FontStyle {
    /// regular text in a colour
    pub fn regular(color: Clr) -> FontStyle {
        FontStyle { style: 0, color }
    }
    /// bold text in a colour
    pub fn bold(color: Clr) -> FontStyle {
        FontStyle { style: 1, color }
    }
}

/// a format scheme: the style lists a QuickStyle matrix number picks from (shapes: six of
/// each, connectors: three)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Scheme {
    /// the name (fmtScheme@name)
    pub name: String,
    pub fills: Vec<Fill>,
    pub lines: Vec<Line>,
    pub effects: Vec<Effect>,
    /// the background fills (bgFillStyleLst, three)
    pub backgrounds: Vec<Fill>,
    pub fonts: Vec<FontStyle>,
}

/// the style a variant gives its quick style 1 .. 4: one-based indices into the lists
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VarStyle {
    pub fill: u8,
    pub line: u8,
    pub effect: u8,
    pub font: u8,
}

impl VarStyle {
    /// fill, line, effect, font
    pub const fn new(fill: u8, line: u8, effect: u8, font: u8) -> VarStyle {
        VarStyle { fill, line, effect, font }
    }
}

/// a variant (TH-04): seven colours and four quick styles
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Variant {
    /// varColor1 .. 7 (0xRRGGBB)
    pub colors: [u32; 7],
    /// one colour for everything (THEMEPROP("MultiFormat") false)
    pub monotone: bool,
    /// the embellishment level 1 .. 3 (low, medium, high)
    pub embellishment: u8,
    /// quick styles 1 .. 4 (QuickStyle matrices 100 .. 103)
    pub styles: [VarStyle; 4],
}

/// a design (TH-01)
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Design {
    /// the theme's name (a:theme@name; JFDiagrammer shows its own translated names)
    pub name: String,
    /// the scheme number of every component
    pub id: u32,
    /// dk1, lt1, dk2, lt2, accent1 .. 6, hlink, folHlink (0xRRGGBB)
    pub colors: [u32; 12],
    /// vt:bkgnd: the page colour themed text keeps its contrast to
    pub background: u32,
    /// the Latin typefaces for headings and body
    pub major_font: String,
    pub minor_font: String,
    /// the format scheme of shapes (six styles of each kind)
    pub shapes: Scheme,
    /// the connector scheme (three of each kind)
    pub connectors: Scheme,
    /// the four variants
    pub variants: [Variant; 4],
}

impl Design {
    /// the theme part's bytes
    pub fn theme_xml(&self) -> Vec<u8> {
        xml::write(self).into_bytes()
    }

    /// the colour a QuickStyle colour number names in a variant (None: not a colour number)
    pub fn quick_color(&self, n: u32, variant: usize) -> Option<u32> {
        match n {
            0 => Some(self.colors[0]),
            1 => Some(self.colors[1]),
            2..=7 => Some(self.colors[(n + 2) as usize]),
            8 => Some(self.background),
            100..=106 => self.variants.get(variant).map(|v| v.colors[(n - 100) as usize]),
            _ => None,
        }
    }
}
