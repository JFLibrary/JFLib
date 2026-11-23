// cells -- the sections, rows and cells of a binary object (shape, style, sheet) as the XML of
// Visio 2003-2010 writes them (vss/mod.rs says what the format is)
//
// An object's sections are lists (chunks with child IDs): 0x68 holds the single-row sections
// (XForm, Line ...), 0x6C a Geometry section, 0x69 the Character rows, 0x6A Paragraph, 0x71
// Connection, 0x70 Controls, 0x67 User, 0x6D Shape Data, 0x6E Actions, 0x64 Scratch, 0x65 a
// group's subshapes; 0x0E is the text, 0x2C the object's strings, 0x42 its unique ID.  The
// object's own child IDs give the order of its Geometry sections.  A list's child IDs give the
// order of its rows; a name index (0xC9) in a list names its rows (into the document's name list).
//
// A row chunk holds its cells in fixed fields (LAYOUTS), a byte 1, a mask with a bit per cell
// that holds a value of its own (no formula), and then blocks: [u32 length][u8 kind][u8 cell]
// [...]; kind 2 is the cell's formula (formula.rs), the others (references between cells) are
// recomputed by whoever needs them and left out here.  A cell with a formula block is written
// with its formula, one with its mask bit with its value alone, any other is inherited and not
// written.  A cell's number in masks and blocks is its index in Visio's row (the order of the XML
// schema, with gaps where Visio has cells the XML does not).  A string cell's field is an index
// into the object's strings (negative: empty).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::stream::{f64at, u16at, u32at};
use super::{formula, utf16z, Bin, Obj, R};

/// a shape's or style's references to its styles
#[derive(Clone, Copy, Default)]
pub(super) struct Refs {
    pub line: Option<u32>,
    pub fill: Option<u32>,
    pub text: Option<u32>,
}

const NONE: u32 = 0xFFFF_FFFF;

// the types of an object's lists
const L_PROPS: u32 = 0x68;
const L_CHAR: u32 = 0x69;
const L_PARA: u32 = 0x6A;
const L_TABS: u32 = 0x6B;
const L_GEOM: u32 = 0x6C;
const L_PROP: u32 = 0x6D;
const L_ACT: u32 = 0x6E;
const L_LAYER: u32 = 0x6F;
const L_CTRL: u32 = 0x70;
const L_CONN: u32 = 0x71;
const L_USER: u32 = 0x67;
const L_FIELD: u32 = 0x66;
const L_SCRATCH: u32 = 0x64;
const L_SHAPES: u32 = 0x65;
const C_TEXT: u32 = 0x0E;
const C_STRINGS: u32 = 0x2C;
const C_STRING: u32 = 0x2D;
const C_NAMES: u32 = 0xC9;

// the header of an object's list (the bytes between the list's lengths and its child IDs)
fn head<'s>(o: &Obj, s: &'s [u8]) -> &'s [u8] {
    o.c.list_parts(s).map_or(&[], |(h, _)| h)
}

/// the styles an object names in its header
pub(super) fn style_refs(o: &Obj, s: &[u8]) -> Refs {
    let h = head(o, s);
    let get = |at: usize| u32at(h, at).filter(|&v| v != NONE);
    Refs { fill: get(26), line: get(34), text: get(42) }
}

pub(super) fn refs_attrs(out: &mut super::Out, r: Refs) {
    if let Some(v) = r.line {
        out.attr("LineStyle", &format!("{v}"));
    }
    if let Some(v) = r.fill {
        out.attr("FillStyle", &format!("{v}"));
    }
    if let Some(v) = r.text {
        out.attr("TextStyle", &format!("{v}"));
    }
}

/// the group a shape belongs to (0: none)
pub(super) fn shape_parent(o: &Obj, s: &[u8]) -> u32 {
    u32at(head(o, s), 2).unwrap_or(0)
}

/// the IDs of a group's subshapes in their order (its list of type 0x65)
pub(super) fn group_children(o: &Obj, s: &[u8]) -> Vec<u32> {
    let mut v = Vec::new();
    for k in o.kids.iter().filter(|k| k.c.ty == L_SHAPES) {
        for r in &k.kids {
            if r.c.ty == 0x83 {
                if let Some(id) = u32at(r.c.data(s), 0) {
                    v.push(id);
                }
            }
        }
    }
    v
}

/// the unique ID of a shape (0x42: a GUID as Windows stores it)
pub(super) fn unique_id(o: &Obj, s: &[u8]) -> Option<String> {
    let k = o.kids.iter().find(|k| k.c.ty == 0x42)?;
    guid(k.c.data(s))
}

/// a GUID of 16 bytes (Data1..3 little-endian) as {XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}
pub(super) fn guid(b: &[u8]) -> Option<String> {
    let b = b.get(..16)?;
    if b.iter().all(|&x| x == 0) {
        return None;
    }
    let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d2 = u16::from_le_bytes([b[4], b[5]]);
    let d3 = u16::from_le_bytes([b[6], b[7]]);
    Some(format!(
        "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

// ---- the table of row layouts

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum K {
    /// a unit byte and a double
    Num,
    /// a double without unit
    Dbl,
    /// a byte, 0 or 1
    Bool,
    /// one bit of a byte (the bit's number in `Field::bit`)
    Bit,
    /// a byte
    U8,
    /// two bytes
    U16,
    /// a colour: index byte, R, G, B (the transparency byte after them is a cell of its own, Trans)
    Color,
    /// a transparency byte (0..255 -> 0..1)
    Trans,
    /// a font: its index (two bytes) is the FaceName ID
    Font,
    /// two bytes, ten thousandths (FontScale)
    Per10k,
    /// a string: the index (i32) of one of the object's strings, negative for none
    Str,
    /// no field: the value 0 (cells that hold only a formula, the events)
    Zero,
}

/// one cell of a row: its name, kind, byte offset and its number in masks and blocks
#[derive(Clone, Copy)]
pub(super) struct Field {
    pub name: &'static str,
    pub k: K,
    pub at: usize,
    pub ix: u8,
    pub bit: u8,
}

const fn f(name: &'static str, k: K, at: usize, ix: u8) -> Field {
    Field { name, k, at, ix, bit: 0 }
}

const fn bit(name: &'static str, at: usize, bit: u8, ix: u8) -> Field {
    Field { name, k: K::Bit, at, ix, bit }
}

pub(super) struct Layout {
    pub ty: u32,
    /// the element: a single-row section, or a row of a section
    pub elem: &'static str,
    /// the bytes of the fixed fields (the byte 1 follows)
    pub fixed: usize,
    /// the bytes of the mask after the byte 1
    pub mask: usize,
    pub fields: &'static [Field],
}

use K::*;

const fn lay(ty: u32, elem: &'static str, fixed: usize, mask: usize, fields: &'static [Field]) -> Layout {
    Layout { ty, elem, fixed, mask, fields }
}

pub(super) const LAYOUTS: &[Layout] = &[
    // ---- single-row sections (in the list 0x68)
    lay(0x9B, "XForm", 66, 2, &[
        f("PinX", Num, 0, 0), f("PinY", Num, 9, 1), f("Width", Num, 18, 2), f("Height", Num, 27, 3),
        f("LocPinX", Num, 36, 4), f("LocPinY", Num, 45, 5), f("Angle", Num, 54, 6),
        f("FlipX", Bool, 63, 7), f("FlipY", Bool, 64, 8), f("ResizeMode", U8, 65, 9),
    ]),
    lay(0x9D, "XForm1D", 36, 1, &[f("BeginX", Num, 0, 0), f("BeginY", Num, 9, 1), f("EndX", Num, 18, 2), f("EndY", Num, 27, 3)]),
    lay(0x9C, "TextXForm", 66, 2, &[
        f("TxtPinX", Num, 0, 0), f("TxtPinY", Num, 9, 1), f("TxtWidth", Num, 18, 2), f("TxtHeight", Num, 27, 3),
        f("TxtLocPinX", Num, 36, 4), f("TxtLocPinY", Num, 45, 5), f("TxtAngle", Num, 54, 6),
    ]),
    lay(0x85, "Line", 30, 4, &[
        f("LineWeight", Num, 0, 0), f("LineColor", Color, 9, 1), f("LinePattern", U8, 14, 2), f("Rounding", Num, 15, 3),
        f("EndArrowSize", U8, 24, 4), f("BeginArrow", U8, 25, 5), f("EndArrow", U8, 26, 6), f("LineCap", U8, 27, 7),
        f("BeginArrowSize", U8, 28, 8), f("LineColorTrans", Trans, 13, 9),
    ]),
    lay(0x86, "Fill", 58, 2, &[
        f("FillForegnd", Color, 0, 0), f("FillBkgnd", Color, 5, 1), f("FillPattern", U8, 10, 2),
        f("ShdwForegnd", Color, 11, 3), f("ShdwBkgnd", Color, 16, 4), f("ShdwPattern", U8, 21, 5),
        f("FillForegndTrans", Trans, 4, 6), f("FillBkgndTrans", Trans, 9, 7), f("ShdwForegndTrans", Trans, 15, 8),
        f("ShdwBkgndTrans", Trans, 20, 9), f("ShapeShdwType", U8, 22, 10), f("ShapeShdwOffsetX", Num, 23, 11),
        f("ShapeShdwOffsetY", Num, 32, 12), f("ShapeShdwObliqueAngle", Num, 41, 13), f("ShapeShdwScaleFactor", Dbl, 50, 14),
    ]),
    lay(0x87, "TextBlock", 87, 4, &[
        f("LeftMargin", Num, 0, 0), f("RightMargin", Num, 9, 1), f("TopMargin", Num, 18, 2), f("BottomMargin", Num, 27, 3),
        f("VerticalAlign", U8, 36, 4), f("TextBkgnd", Color, 37, 5), f("DefaultTabStop", Num, 42, 6),
        f("TextBkgndTrans", Trans, 41, 11),
    ]),
    lay(0xA0, "Protection", 20, 3, &[
        f("LockWidth", Bool, 0, 0), f("LockHeight", Bool, 1, 1), f("LockMoveX", Bool, 2, 2), f("LockMoveY", Bool, 3, 3),
        f("LockAspect", Bool, 4, 4), f("LockDelete", Bool, 5, 5), f("LockBegin", Bool, 6, 6), f("LockEnd", Bool, 7, 7),
        f("LockRotate", Bool, 8, 8), f("LockCrop", Bool, 9, 9), f("LockVtxEdit", Bool, 10, 10), f("LockTextEdit", Bool, 11, 11),
        f("LockFormat", Bool, 12, 12), f("LockGroup", Bool, 13, 13), f("LockCalcWH", Bool, 14, 14), f("LockSelect", Bool, 15, 15),
        f("LockCustProp", Bool, 16, 16), f("LockFromGroupFormat", Bool, 17, 17), f("LockThemeColors", Bool, 18, 18),
        f("LockThemeEffects", Bool, 19, 19),
    ]),
    lay(0x93, "StyleProp", 4, 2, &[
        f("EnableLineProps", Bool, 0, 0), f("EnableFillProps", Bool, 1, 1), f("EnableTextProps", Bool, 2, 2), f("HideForApply", Bool, 3, 3),
    ]),
    lay(0x84, "Event", 12, 4, &[
        f("TheData", Zero, 0, 0), f("TheText", Zero, 0, 1), f("EventDblClick", Zero, 0, 2), f("EventXFMod", Zero, 0, 3), f("EventDrop", Zero, 0, 4),
    ]),
    lay(0xA7, "LayerMem", 4, 1, &[f("LayerMember", Str, 0, 0)]),
    lay(0xA3, "Help", 28, 1, &[f("HelpTopic", Str, 0, 0), f("Copyright", Str, 4, 1)]),
    lay(0xBE, "Group", 20, 4, &[
        f("SelectMode", U8, 0, 0), f("DisplayMode", U8, 1, 1), bit("IsDropTarget", 2, 0, 2), bit("IsSnapTarget", 2, 1, 3),
        bit("IsTextEditTarget", 2, 2, 4), bit("DontMoveChildren", 2, 3, 5),
    ]),
    lay(0xA4, "Misc", 40, 4, &[
        bit("NoObjHandles", 0, 0, 0), bit("NonPrinting", 0, 1, 1), bit("NoCtlHandles", 0, 2, 2), bit("NoAlignBox", 0, 3, 3),
        bit("UpdateAlignBox", 0, 4, 4), bit("HideText", 0, 5, 5), f("DynFeedback", U8, 1, 8), f("GlueType", U8, 2, 9),
        f("WalkPreference", U8, 3, 10), f("BegTrigger", U8, 4, 11), f("EndTrigger", U8, 5, 12), f("ObjType", U8, 6, 13),
        f("Comment", Str, 12, 16), f("LangID", U16, 18, 26), f("ShapeKeywords", Str, 20, 27), f("DropOnPageScale", Dbl, 24, 28),
    ]),
    lay(0x92, "PageProps", 125, 6, &[
        f("PageWidth", Num, 0, 0), f("PageHeight", Num, 9, 1), f("ShdwOffsetX", Num, 18, 2), f("ShdwOffsetY", Num, 27, 3),
        f("PageScale", Num, 36, 4), f("DrawingScale", Num, 45, 5), f("DrawingSizeType", U8, 54, 6), f("DrawingScaleType", U8, 55, 7),
    ]),
    // ---- rows of sections
    lay(0x94, "Char", 79, 8, &[
        f("Font", Font, 4, 0), f("Color", Color, 6, 1), f("Style", U8, 11, 2), f("Case", U8, 12, 3), f("Pos", U8, 13, 4),
        f("FontScale", Per10k, 14, 5), f("Size", Num, 17, 7), f("ColorTrans", Trans, 10, 17), f("Letterspace", Num, 51, 16),
        f("ComplexScriptSize", Num, 42, 54), f("LangID", U16, 69, 57),
    ]),
    lay(0x95, "Para", 118, 4, &[
        f("IndFirst", Num, 4, 0), f("IndLeft", Num, 13, 1), f("IndRight", Num, 22, 2), f("SpLine", Num, 31, 3),
        f("SpBefore", Num, 40, 4), f("SpAfter", Num, 49, 5), f("HorzAlign", U8, 58, 6), f("Bullet", U8, 59, 7),
        f("BulletStr", Str, 60, 8), f("BulletFontSize", Num, 67, 11), f("TextPosAfterBullet", Num, 76, 12),
    ]),
    lay(0x99, "Connection", 44, 3, &[
        f("X", Num, 0, 0), f("Y", Num, 9, 1), f("DirX", Num, 18, 2), f("DirY", Num, 27, 3), f("Type", U8, 36, 4),
        f("AutoGen", Bool, 37, 5), f("Prompt", Str, 38, 6),
    ]),
    lay(0xAA, "Control", 44, 2, &[
        f("X", Num, 0, 0), f("Y", Num, 9, 1), f("XDyn", Num, 18, 2), f("YDyn", Num, 27, 3), f("XCon", U8, 36, 4),
        f("YCon", U8, 37, 5), f("CanGlue", Bool, 38, 6), f("Prompt", Str, 40, 8),
    ]),
    lay(0xB4, "User", 13, 1, &[f("Value", Num, 0, 0), f("Prompt", Str, 9, 1)]),
    lay(0xA9, "Act", 52, 4, &[
        f("Menu", Str, 0, 0), f("Action", Num, 12, 3), f("Checked", U8, 21, 4), f("Disabled", U8, 22, 5), f("ReadOnly", U8, 23, 6),
        f("Invisible", U8, 36, 7), f("BeginGroup", U8, 37, 8), f("TagName", Str, 24, 9), f("ButtonFace", Str, 28, 10),
        f("SortKey", Str, 32, 11), f("FlyoutChild", U8, 38, 12),
    ]),
    lay(0xA1, "Field", 51, 2, &[
        f("Value", Num, 7, 0), f("Format", Num, 17, 2), f("Type", U8, 26, 3), f("UICat", U8, 28, 4), f("UICod", U8, 29, 5),
        f("UIFmt", U8, 30, 6), f("Calendar", U8, 31, 7), f("ObjectKind", U8, 32, 8),
    ]),
    lay(0xB6, "Prop", 31, 4, &[
        f("Value", Num, 0, 0), f("Prompt", Str, 9, 1), f("Label", Str, 13, 2), f("Format", Str, 17, 3), f("SortKey", Str, 21, 4),
        f("Type", U8, 25, 5), f("Invisible", Bool, 26, 6), f("Verify", Bool, 27, 7), f("LangID", U16, 28, 14),
        f("Calendar", U8, 30, 15),
    ]),
    lay(0x9E, "Scratch", 54, 1, &[
        f("X", Num, 0, 0), f("Y", Num, 9, 1), f("A", Num, 18, 2), f("B", Num, 27, 3), f("C", Num, 36, 4), f("D", Num, 45, 5),
    ]),
    // ---- Geometry: the section's own cells and its rows
    lay(0x89, "", 1, 1, &[
        bit("NoFill", 0, 0, 0), bit("NoLine", 0, 1, 1), bit("NoShow", 0, 2, 2), bit("NoSnap", 0, 3, 3), bit("NoQuickDrag", 0, 4, 4),
    ]),
    lay(0x8A, "MoveTo", 18, 1, &[f("X", Num, 0, 0), f("Y", Num, 9, 1)]),
    lay(0x8B, "LineTo", 18, 1, &[f("X", Num, 0, 0), f("Y", Num, 9, 1)]),
    lay(0x8C, "ArcTo", 27, 1, &[f("X", Num, 0, 0), f("Y", Num, 9, 1), f("A", Num, 18, 2)]),
    lay(0x8F, "Ellipse", 54, 1, &[
        f("X", Num, 0, 0), f("Y", Num, 9, 1), f("A", Num, 18, 2), f("B", Num, 27, 3), f("C", Num, 36, 4), f("D", Num, 45, 5),
    ]),
    lay(0x90, "EllipticalArcTo", 54, 1, &[
        f("X", Num, 0, 0), f("Y", Num, 9, 1), f("A", Num, 18, 2), f("B", Num, 27, 3), f("C", Num, 36, 4), f("D", Num, 45, 5),
    ]),
];

fn layout(ty: u32) -> Option<&'static Layout> {
    LAYOUTS.iter().find(|l| l.ty == ty)
}

/// the name the XML gives a unit code of the binary format (Visio's unit codes)
pub(super) fn unit_name(code: u8) -> Option<&'static str> {
    Some(match code {
        0x21 => "PER",
        // drawing units, default angles and text units are each cell's default: Visio writes no U
        0x30 | 0x40 | 0x50 => return None,
        0x24 => "AC",
        0x25 => "HA",
        0x28 => "DATE",
        0x32 => "PT",
        0x33 => "P",
        0x34 => "P_PT",
        0x35 => "D",
        0x36 => "C",
        0x37 => "C_D",
        0x3F => "DP",
        0x41 => "IN",
        0x42 => "FT",
        0x43 => "F_I",
        0x44 => "MI",
        0x45 => "CM",
        0x46 => "MM",
        0x47 => "M",
        0x48 => "KM",
        0x49 => "IN_F",
        0x4A => "MI_F",
        0x4B => "YD",
        0x4C => "NM",
        0x51 => "DEG",
        0x52 => "AD",
        0x53 => "RAD",
        _ => return None,
    })
}

// a double as Visio writes a value: at most 16 significant digits (0.01 stays 0.01 although
// the double is 0.009999999999999998), E notation below 1E-5 and from 1E16; -0 is 0 (the VSDX
// writer writes it so, and a round trip must not tell them apart)
pub(super) fn num(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return String::from("0");
    }
    let sci = format!("{:e}", v.abs());
    let e: i32 = sci.split_once('e').and_then(|(_, e)| e.parse().ok()).unwrap_or(0);
    if !(-5..16).contains(&e) {
        let m = format!("{:.15e}", v);
        let (mant, ex) = m.split_once('e').unwrap_or((m.as_str(), "0"));
        let mant = trim_zeros(mant);
        return format!("{mant}E{ex}");
    }
    let d = (15 - e).max(0) as usize;
    let t = trim_zeros(&format!("{:.*}", d, v));
    if t == "-0" {
        String::from("0")
    } else {
        t
    }
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') {
        String::from(s.trim_end_matches('0').trim_end_matches('.'))
    } else {
        String::from(s)
    }
}

/// the unit code even where it is a cell's default (Geometry and Scratch A..E have no default
/// unit: Visio writes DL and DA there)
fn unit_code(code: u8) -> Option<&'static str> {
    match code {
        0x30 => Some("DT"),
        0x40 => Some("DL"),
        0x50 => Some("DA"),
        _ => unit_name(code),
    }
}

/// what a cell's value field needs from its object: the strings
pub(super) struct Ctx<'a> {
    pub strings: &'a [(u32, String)],
}

impl Ctx<'_> {
    fn string(&self, ix: i32) -> String {
        if ix < 0 {
            return String::new();
        }
        self.strings.iter().find(|(i, _)| *i == ix as u32).map(|(_, s)| s.clone()).unwrap_or_default()
    }
}

/// the value of a field as XML text and its unit
fn value(d: &[u8], fl: &Field, ctx: &Ctx) -> Option<(String, Option<&'static str>)> {
    let at = fl.at;
    Some(match fl.k {
        Num => {
            let u = *d.get(at)?;
            match u {
                // a truth value, a string (an index into the object's strings), a colour index
                0x61 => (format!("{}", u32at(d, at + 1)?), Some("BOOL")),
                0xE8 => (ctx.string(u32at(d, at + 1)? as i32), Some("STR")),
                0xFB => (format!("{}", u32at(d, at + 1)?), Some("COLOR")),
                _ => {
                    let keep = matches!(fl.name, "A" | "B" | "C" | "D" | "E");
                    (num(f64at(d, at + 1)?), if keep { unit_code(u) } else { unit_name(u) })
                }
            }
        }
        Dbl => (num(f64at(d, at)?), None),
        Bool => (format!("{}", (*d.get(at)? != 0) as u8), None),
        Bit => (format!("{}", (*d.get(at)? >> fl.bit) & 1), None),
        U8 => (format!("{}", *d.get(at)?), None),
        U16 => (format!("{}", u16at(d, at)?), None),
        Color => {
            let ix = *d.get(at)?;
            let rgb = d.get(at + 1..at + 4)?;
            // the document's 24 colours by index, any other by its RGB
            if ix >= 24 {
                (format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]), None)
            } else {
                (format!("{ix}"), None)
            }
        }
        Trans => (num(*d.get(at)? as f64 / 255.0), None),
        Font => (format!("{}", u16at(d, at)?), None),
        Per10k => (num(u16at(d, at)? as f64 / 10000.0), None),
        Str => (ctx.string(u32at(d, at)? as i32), None),
        Zero => (String::from("0"), None),
    })
}

/// the formula blocks of a row: (cell number, tokens)
fn blocks(d: &[u8], from: usize) -> Vec<(u8, &[u8])> {
    let mut v = Vec::new();
    let mut i = from;
    while i + 6 <= d.len() {
        let len = u32at(d, i).unwrap_or(0) as usize;
        if len < 6 || i + len > d.len() {
            break;
        }
        if d[i + 4] == 2 {
            v.push((d[i + 5], &d[i + 6..i + len]));
        }
        i += len;
    }
    v
}

/// how a row's cells without formula and without their mask bit are written
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unset {
    /// inherited (from the style or the master): not written
    Inherit,
    /// a value of its own without formula: written with the formula "No Formula" (the rows of
    /// sections that no style gives, Geometry, Connection, User ...)
    NoFormula,
}

/// write one row's cells
fn row_cells(bin: &mut Bin, l: &Layout, d: &[u8], ctx: &Ctx, fcx: &formula::Ctx, unset: Unset) -> R<()> {
    let mask = d.get(l.fixed + 1..l.fixed + 1 + l.mask).unwrap_or(&[]);
    let bl = blocks(d, l.fixed + 1 + l.mask);
    for fl in l.fields {
        let block = bl.iter().find(|b| b.0 == fl.ix);
        let local = mask.get(fl.ix as usize / 8).is_some_and(|m| m & (1 << (fl.ix % 8)) != 0);
        if block.is_none() && !local && unset == Unset::Inherit {
            continue;
        }
        let Some((mut v, u)) = value(d, fl, ctx) else { continue };
        let mut f = block.and_then(|b| formula::decode(b.1, fcx));
        if block.is_some() && f.is_none() {
            bin.warn(String::from("formula"));
        }
        // a literal is the value itself (Visio writes no formula for it); a string literal also
        // gives a string cell its value
        if let Some(x) = f.as_deref() {
            if formula::is_literal(x) {
                if let Some(t) = formula::literal_text(x) {
                    if fl.k == Str || v.is_empty() {
                        v = t;
                    }
                }
                f = None;
            }
        }
        if unset == Unset::Inherit && f.is_none() && v.is_empty() && matches!(fl.k, Str) {
            continue;
        }
        let f = match (&f, block, local) {
            (None, None, false) => Some(String::from("No Formula")),
            _ => f,
        };
        bin.out.cell(fl.name, &v, u, f.as_deref())?;
    }
    Ok(())
}

// the VDX element of a list of rows and whether its rows are named
fn rows_elem(ty: u32) -> Option<(&'static str, u8, bool)> {
    // (row element, Visio's section index for formulas, named)
    Some(match ty {
        L_CHAR => ("Char", 3, false),
        L_PARA => ("Para", 4, false),
        L_SCRATCH => ("Scratch", 6, false),
        L_CONN => ("Connection", 7, false),
        L_CTRL => ("Control", 9, true),
        L_ACT => ("Act", 240, true),
        L_USER => ("User", 242, true),
        L_PROP => ("Prop", 243, true),
        L_FIELD => ("Field", 8, false),
        _ => return None,
    })
}

// the rows of a list in the order of its child IDs (rows the list does not name come after)
fn ordered<'o>(sec: &'o Obj, s: &[u8]) -> Vec<&'o Obj> {
    let ids = sec.c.list_parts(s).map(|(_, ids)| ids).unwrap_or_default();
    let rows: Vec<&Obj> = sec.kids.iter().filter(|k| k.c.ty != C_NAMES).collect();
    let mut out: Vec<&Obj> = Vec::new();
    for id in ids {
        if let Some(r) = rows.iter().find(|r| r.c.id == id) {
            if !out.iter().any(|o| core::ptr::eq(*o, *r)) {
                out.push(r);
            }
        }
    }
    for r in rows {
        if !out.iter().any(|o| core::ptr::eq(*o, r)) {
            out.push(r);
        }
    }
    out
}

// a name index (0xC9) of a list: (row ID, Name, NameU)
fn list_names(names: &[String], sec: &Obj, s: &[u8]) -> Vec<(u32, Option<String>, Option<String>)> {
    let mut v = Vec::new();
    for k in sec.kids.iter().filter(|k| k.c.ty == C_NAMES) {
        let d = k.c.data(s);
        let n = u32at(d, 0).unwrap_or(0) as usize;
        for i in 0..n.min(d.len() / 13) {
            let (Some(a), Some(b), Some(r)) = (u32at(d, 4 + 13 * i), u32at(d, 8 + 13 * i), u32at(d, 12 + 13 * i)) else { break };
            let get = |i: u32| names.get(i as usize).filter(|n| !n.is_empty()).cloned();
            v.push((r, get(a), get(b)));
        }
    }
    v
}

/// the object's strings (0x2C with its 0x2D children: their IDs are the indices)
fn strings(o: &Obj, s: &[u8]) -> Vec<(u32, String)> {
    let mut v = Vec::new();
    for k in o.kids.iter().filter(|k| k.c.ty == C_STRINGS) {
        for r in k.kids.iter().filter(|r| r.c.ty == C_STRING) {
            v.push((r.c.id, utf16z(r.c.data(s))));
        }
    }
    v
}

/// the sections of an object, then its text
pub(super) fn sheet(bin: &mut Bin, o: &Obj, s: &[u8], own: u32) -> R<()> {
    let strs = strings(o, s);
    let ctx = Ctx { strings: &strs };
    let names = core::mem::take(&mut bin.names);
    let r = sheet_with(bin, o, s, &ctx, &formula::Ctx { names: &names, own });
    bin.names = names;
    r
}

fn sheet_with(bin: &mut Bin, o: &Obj, s: &[u8], ctx: &Ctx, fcx: &formula::Ctx) -> R<()> {
    // the order of the Geometry sections: the object's own child IDs
    let geo_order: Vec<u32> = o.c.list_parts(s).map(|(_, ids)| ids).unwrap_or_default();
    let mut geoms: Vec<&Obj> = o.kids.iter().filter(|k| k.c.ty == L_GEOM).collect();
    geoms.sort_by_key(|g| geo_order.iter().position(|&id| id == g.c.id).unwrap_or(usize::MAX));
    for sec in &o.kids {
        if sec.c.ty == L_PROPS {
            for r in ordered(sec, s) {
                let Some(l) = layout(r.c.ty) else {
                    bin.warn(format!("cells 0x{:02X}", r.c.ty));
                    continue;
                };
                bin.out.raw("<")?;
                bin.out.raw(l.elem)?;
                bin.out.raw(">")?;
                row_cells(bin, l, r.c.data(s), ctx, fcx, Unset::Inherit)?;
                bin.out.end(l.elem)?;
            }
        }
    }
    for (gi, sec) in geoms.iter().enumerate() {
        bin.out.open("Geom");
        bin.out.attr("IX", &format!("{gi}"));
        bin.out.close()?;
        for r in ordered(sec, s) {
            let Some(l) = layout(r.c.ty) else {
                bin.warn(format!("geometry row 0x{:02X}", r.c.ty));
                continue;
            };
            if l.elem.is_empty() {
                row_cells(bin, l, r.c.data(s), ctx, fcx, Unset::NoFormula)?;
                continue;
            }
            bin.out.open(l.elem);
            bin.out.attr("IX", &format!("{}", r.c.id));
            bin.out.close()?;
            row_cells(bin, l, r.c.data(s), ctx, fcx, Unset::NoFormula)?;
            bin.out.end(l.elem)?;
        }
        bin.out.end("Geom")?;
    }
    for sec in &o.kids {
        let Some((elem, _, is_named)) = rows_elem(sec.c.ty) else {
            if !matches!(sec.c.ty, L_PROPS | L_GEOM | L_SHAPES | C_TEXT | C_STRINGS | 0x42 | L_TABS | L_LAYER) {
                bin.warn(format!("section 0x{:02X}", sec.c.ty));
            }
            continue;
        };
        let rnames = if is_named { list_names(fcx.names, sec, s) } else { Vec::new() };
        for (k, r) in ordered(sec, s).into_iter().enumerate() {
            let Some(l) = layout(r.c.ty) else {
                bin.warn(format!("row 0x{:02X}", r.c.ty));
                continue;
            };
            bin.out.open(elem);
            if is_named {
                let (name, name_u) = rnames.iter().find(|n| n.0 == r.c.id).map_or((None, None), |n| (n.1.clone(), n.2.clone()));
                let nu = name_u.clone().or(name.clone()).unwrap_or_else(|| format!("Row_{}", r.c.id));
                bin.out.attr("NameU", &nu);
                if let Some(n) = name.filter(|n| *n != nu) {
                    bin.out.attr("Name", &n);
                }
                bin.out.attr("ID", &format!("{}", r.c.id));
            } else {
                bin.out.attr("IX", &format!("{k}"));
            }
            bin.out.close()?;
            // Character and Paragraph rows inherit from the text style like single-row sections
            let unset = if matches!(sec.c.ty, L_CHAR | L_PARA) { Unset::Inherit } else { Unset::NoFormula };
            row_cells(bin, l, r.c.data(s), ctx, fcx, unset)?;
            bin.out.end(elem)?;
        }
    }
    text(bin, o, s, ctx)
}

// the text: [8 bytes][UTF-16, NUL-terminated]; the Character and Paragraph rows say how many
// characters each runs over (the first word of their data; 0: to the end)
fn text(bin: &mut Bin, o: &Obj, s: &[u8], ctx: &Ctx) -> R<()> {
    let Some(t) = o.kids.iter().find(|k| k.c.ty == C_TEXT) else { return Ok(()) };
    let d = t.c.data(s);
    let units: Vec<u16> = d.get(8..).unwrap_or(&[]).chunks_exact(2).map(|x| u16::from_le_bytes([x[0], x[1]])).take_while(|&u| u != 0).collect();
    // the fields' values (a Field row's Value: a typed number holding a string index), in order:
    // each U+FFFC of the text stands for the next one
    let mut fields: Vec<String> = Vec::new();
    for sec in o.kids.iter().filter(|k| k.c.ty == L_FIELD) {
        for r in ordered(sec, s) {
            let d = r.c.data(s);
            let v = match d.get(7) {
                Some(0xE8) => ctx.string(u32at(d, 8).unwrap_or(u32::MAX) as i32),
                Some(_) => f64at(d, 8).map(num).unwrap_or_default(),
                None => String::new(),
            };
            fields.push(v);
        }
    }
    let runs = |ty: u32| -> Vec<usize> {
        let mut v = Vec::new();
        let mut at = 0usize;
        for sec in o.kids.iter().filter(|k| k.c.ty == ty) {
            for r in ordered(sec, s) {
                v.push(at);
                let n = u32at(r.c.data(s), 0).unwrap_or(0) as usize;
                at = at.saturating_add(if n == 0 { usize::MAX / 4 } else { n });
            }
        }
        v
    };
    let cps = runs(L_CHAR);
    let pps = runs(L_PARA);
    let tps = runs(L_TABS);
    if units.is_empty() {
        // an empty text keeps the marks of its first runs (Visio writes <Text><cp IX='0'/></Text>)
        if cps.is_empty() && pps.is_empty() && tps.is_empty() {
            return bin.out.raw("<Text/>");
        }
        bin.out.raw("<Text>")?;
        for (list, tag) in [(&cps, "cp"), (&pps, "pp"), (&tps, "tp")] {
            if !list.is_empty() {
                bin.out.raw(&format!("<{tag} IX='0'/>"))?;
            }
        }
        return bin.out.raw("</Text>");
    }
    bin.out.raw("<Text>")?;
    let mut buf: Vec<u16> = Vec::new();
    let flush = |bin: &mut Bin, buf: &mut Vec<u16>| -> R<()> {
        if !buf.is_empty() {
            let t = String::from_utf16_lossy(buf);
            bin.out.text(&t)?;
            buf.clear();
        }
        Ok(())
    };
    for (i, &u) in units.iter().enumerate() {
        let cp = cps.iter().position(|&p| p == i);
        let pp = pps.iter().position(|&p| p == i);
        let tp = tps.iter().position(|&p| p == i);
        if cp.is_some() || pp.is_some() || tp.is_some() {
            flush(bin, &mut buf)?;
            for (m, tag) in [(cp, "cp"), (pp, "pp"), (tp, "tp")] {
                if let Some(k) = m {
                    bin.out.raw(&format!("<{tag} IX='{k}'/>"))?;
                }
            }
        }
        if u == 0xFFFC {
            flush(bin, &mut buf)?;
            let k = cps_fields(&units[..i]);
            match fields.get(k).filter(|v| !v.is_empty()) {
                Some(v) => {
                    bin.out.raw(&format!("<fld IX='{k}'>"))?;
                    bin.out.text(v)?;
                    bin.out.raw("</fld>")?;
                }
                // a field without a value is the mark alone (as VSDX reads it back)
                None => bin.out.raw(&format!("<fld IX='{k}'/>"))?,
            }
            continue;
        }
        buf.push(u);
    }
    flush(bin, &mut buf)?;
    bin.out.raw("</Text>")
}

// the number of fields before a place in the text
fn cps_fields(before: &[u16]) -> usize {
    before.iter().filter(|&&u| u == 0xFFFC).count()
}
