// names -- the section, row and cell names the model itself refers to, and the style categories
//
//   names::PIN_X ...                 (crate) fixed Syms: every Strings table starts with them
//   Category::of(section, cell)      which style a cell inherits from: line, fill (with the
//                                    effects) or text; None: only from the master shape
//
// The categories follow the public format description [MS-VSDX] ("Style-to-Shape Inheritance",
// "Line Properties", "Fill Properties", "Effect Properties", "Text"): LineStyle carries the
// line cells and the LineGradient section, FillStyle the fill and effect cells and the
// FillGradient section, TextStyle the Character, Paragraph and Tabs sections and the text block
// cells.  The theme cells (ThemeIndex, ColorSchemeIndex ...) and the quick style cells have no
// category there; shapes take them from the document's "Theme" style, so they go with fill
// (the line and font quick style cells with line and text) -- an assumption the files cannot
// test, since every style in them names one style for all three (JD6).

use crate::strings::Sym;

macro_rules! known {
    ($($id:ident = $s:literal),* $(,)?) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms, dead_code)]
        #[repr(u32)]
        enum K { _Empty = 0, $($id),* }
        /// (crate) the names every string table starts with, in the order of their Syms
        pub(crate) const KNOWN: &[&str] = &[$($s),*];
        $(#[allow(dead_code)] pub(crate) const $id: Sym = Sym(K::$id as u32);)*
    };
}

known! {
    // sections
    GEOMETRY = "Geometry", USER = "User", PROPERTY = "Property", CHARACTER = "Character",
    PARAGRAPH = "Paragraph", TABS = "Tabs", CONTROL = "Control", CONNECTION = "Connection",
    ACTIONS = "Actions", ACTION_TAG = "ActionTag", HYPERLINK = "Hyperlink", LAYER = "Layer",
    FILL_GRADIENT = "FillGradient", LINE_GRADIENT = "LineGradient", SCRATCH = "Scratch",
    FIELD = "Field", REVIEWER = "Reviewer", ANNOTATION = "Annotation",
    // transform, 1D, text transform
    PIN_X = "PinX", PIN_Y = "PinY", WIDTH = "Width", HEIGHT = "Height", LOC_PIN_X = "LocPinX",
    LOC_PIN_Y = "LocPinY", ANGLE = "Angle", FLIP_X = "FlipX", FLIP_Y = "FlipY",
    RESIZE_MODE = "ResizeMode", BEGIN_X = "BeginX", BEGIN_Y = "BeginY", END_X = "EndX",
    END_Y = "EndY", TXT_PIN_X = "TxtPinX", TXT_PIN_Y = "TxtPinY", TXT_WIDTH = "TxtWidth",
    TXT_HEIGHT = "TxtHeight", TXT_LOC_PIN_X = "TxtLocPinX", TXT_LOC_PIN_Y = "TxtLocPinY",
    TXT_ANGLE = "TxtAngle",
    // row cells
    X = "X", Y = "Y", A = "A", B = "B", C = "C", D = "D", E = "E",
    NO_FILL = "NoFill", NO_LINE = "NoLine", NO_SHOW = "NoShow", NO_SNAP = "NoSnap",
    NO_QUICK_DRAG = "NoQuickDrag",
    // line
    LINE_WEIGHT = "LineWeight", LINE_COLOR = "LineColor", LINE_PATTERN = "LinePattern",
    ROUNDING = "Rounding", END_ARROW_SIZE = "EndArrowSize", BEGIN_ARROW = "BeginArrow",
    END_ARROW = "EndArrow", LINE_CAP = "LineCap", BEGIN_ARROW_SIZE = "BeginArrowSize",
    LINE_COLOR_TRANS = "LineColorTrans", COMPOUND_TYPE = "CompoundType",
    LINE_GRADIENT_DIR = "LineGradientDir", LINE_GRADIENT_ANGLE = "LineGradientAngle",
    LINE_GRADIENT_ENABLED = "LineGradientEnabled",
    // fill
    FILL_FOREGND = "FillForegnd", FILL_FOREGND_TRANS = "FillForegndTrans",
    FILL_BKGND = "FillBkgnd", FILL_BKGND_TRANS = "FillBkgndTrans", FILL_PATTERN = "FillPattern",
    FILL_GRADIENT_DIR = "FillGradientDir", FILL_GRADIENT_ANGLE = "FillGradientAngle",
    FILL_GRADIENT_ENABLED = "FillGradientEnabled",
    ROTATE_GRADIENT_WITH_SHAPE = "RotateGradientWithShape", USE_GROUP_GRADIENT = "UseGroupGradient",
    // text block
    LEFT_MARGIN = "LeftMargin", RIGHT_MARGIN = "RightMargin", TOP_MARGIN = "TopMargin",
    BOTTOM_MARGIN = "BottomMargin", VERTICAL_ALIGN = "VerticalAlign", TEXT_BKGND = "TextBkgnd",
    TEXT_BKGND_TRANS = "TextBkgndTrans", DEFAULT_TAB_STOP = "DefaultTabStop",
    TEXT_DIRECTION = "TextDirection",
    // shape, page
    LAYER_MEMBER = "LayerMember", PAGE_WIDTH = "PageWidth", PAGE_HEIGHT = "PageHeight",
    PAGE_SCALE = "PageScale", DRAWING_SCALE = "DrawingScale",
    // layer, shape data, user, hyperlink, connection rows
    NAME = "Name", NAME_UNIV = "NameUniv", VISIBLE = "Visible", PRINT = "Print",
    ACTIVE = "Active", LOCK = "Lock", SNAP = "Snap", GLUE = "Glue", COLOR = "Color",
    COLOR_TRANS = "ColorTrans", STATUS = "Status", LABEL = "Label", PROMPT = "Prompt",
    TYPE = "Type", FORMAT = "Format", VALUE = "Value", SORT_KEY = "SortKey",
    INVISIBLE = "Invisible", VERIFY = "Verify", DESCRIPTION = "Description",
    ADDRESS = "Address", SUB_ADDRESS = "SubAddress", EXTRA_INFO = "ExtraInfo", FRAME = "Frame",
    NEW_WINDOW = "NewWindow", DEFAULT = "Default", DIR_X = "DirX", DIR_Y = "DirY",
    AUTO_GEN = "AutoGen",
    // geometry row types
    MOVE_TO = "MoveTo", LINE_TO = "LineTo", ARC_TO = "ArcTo", ELLIPTICAL_ARC_TO = "EllipticalArcTo",
    ELLIPSE = "Ellipse", NURBS_TO = "NURBSTo", POLYLINE_TO = "PolylineTo",
    SPLINE_START = "SplineStart", SPLINE_KNOT = "SplineKnot", INFINITE_LINE = "InfiniteLine",
    REL_MOVE_TO = "RelMoveTo", REL_LINE_TO = "RelLineTo", REL_CUB_BEZ_TO = "RelCubBezTo",
    REL_QUAD_BEZ_TO = "RelQuadBezTo", REL_ELLIPTICAL_ARC_TO = "RelEllipticalArcTo",
    // structure (JD10): read by the structure functions before any shape has them
    RELATIONSHIPS = "Relationships", MSV_SHAPE_CATEGORIES = "msvShapeCategories",
    MSV_STRUCTURE_TYPE = "msvStructureType",
}

/// which style sheet a cell inherits from (the style attributes LineStyle, FillStyle, TextStyle)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Category {
    /// LineStyle: line cells and the LineGradient section
    Line,
    /// FillStyle: fill and effect cells (shadow, bevel, glow, reflection, soft edges, sketch,
    /// 3D rotation) and the FillGradient section
    Fill,
    /// TextStyle: the Character, Paragraph and Tabs sections and the text block cells
    Text,
}

const LINE_CELLS: &[&str] = &[
    "LineColor", "LinePattern", "LineWeight", "LineCap", "BeginArrow", "EndArrow", "LineColorTrans",
    "CompoundType", "BeginArrowSize", "EndArrowSize", "Rounding", "LineGradientDir",
    "LineGradientAngle", "LineGradientEnabled", "QuickStyleLineColor", "QuickStyleLineMatrix",
];
const FILL_CELLS: &[&str] = &[
    "FillForegnd", "FillForegndTrans", "FillBkgnd", "FillBkgndTrans", "FillPattern", "FillGradientDir",
    "FillGradientAngle", "FillGradientEnabled", "RotateGradientWithShape", "UseGroupGradient",
    "Perspective", "DistanceFromGround", "KeepTextFlat", "SoftEdgesSize",
    // the quick style cells of fill and effects, and the theme cells (JD6: shapes take them from
    // the document's "Theme" style; [MS-VSDX] names no category for them, the files' styles
    // carry them for line, fill and text alike)
    "QuickStyleFillColor", "QuickStyleFillMatrix", "QuickStyleShadowColor", "QuickStyleEffectsMatrix",
    "QuickStyleType", "QuickStyleVariation", "ThemeIndex", "ColorSchemeIndex", "EffectSchemeIndex",
    "ConnectorSchemeIndex", "FontSchemeIndex", "VariationColorIndex", "VariationStyleIndex",
    "EmbellishmentIndex",
];
// effect cells by their family: shadow, bevel, glow, reflection, sketch, 3D rotation
const EFFECT_PREFIXES: &[&str] = &["Shdw", "ShapeShdw", "Bevel", "Glow", "Reflection", "Sketch", "RotationXAngle",
    "RotationYAngle", "RotationZAngle", "RotationType"];
const TEXT_CELLS: &[&str] = &[
    "LeftMargin", "RightMargin", "TopMargin", "BottomMargin", "VerticalAlign", "TextBkgnd", "TextBkgndTrans",
    "DefaultTabStop", "TextDirection", "QuickStyleFontColor", "QuickStyleFontMatrix",
];

impl Category {
    /// the category of a cell; `section` is the name of its section (None: a single cell)
    pub fn of(section: Option<&str>, cell: &str) -> Option<Category> {
        if let Some(sec) = section {
            return Category::of_section(sec);
        }
        if LINE_CELLS.contains(&cell) {
            Some(Category::Line)
        } else if FILL_CELLS.contains(&cell) || EFFECT_PREFIXES.iter().any(|p| cell.starts_with(p)) {
            Some(Category::Fill)
        } else if TEXT_CELLS.contains(&cell) {
            Some(Category::Text)
        } else {
            None
        }
    }

    /// the category of a whole section (None: rows come only from the master shape)
    pub fn of_section(section: &str) -> Option<Category> {
        match section {
            "LineGradient" => Some(Category::Line),
            "FillGradient" => Some(Category::Fill),
            "Character" | "Paragraph" | "Tabs" => Some(Category::Text),
            _ => None,
        }
    }
}

/// whether rows of this section are named (N) rather than indexed (IX), after [MS-VSDX] Row_Type
pub fn named_rows(section: &str) -> bool {
    matches!(section, "User" | "Property" | "Actions" | "Control" | "Hyperlink" | "ActionTag")
}
