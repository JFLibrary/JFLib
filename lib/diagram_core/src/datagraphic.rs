// datagraphic -- (JD19) data graphics: callouts, data bars, icon sets and colours that show a
// shape's data (DA-07, DA-08, ST-08), stored as Visio stores them
//
//   DataGraphic, Item, Kind, Pos, Opt, Rule, Op, Shade   the model of a data graphic: its name,
//                                   its items (a text callout, a data bar, an icon set, a colour by
//                                   value; the field expression "{Label}" or a formula of such
//                                   fields, the callout master, the position or the graphic's
//                                   default, the options, icon rules, colours), the default
//                                   position, Shape-Text ausblenden, Rahmen
//   is_graphic(doc, m) / graphics(doc)   a master is a data graphic: User.visUSEType = 1 in its
//                                   page sheet
//   read(doc, m) -> DataGraphic      the model of a data graphic master (Visio's or ours)
//   create(tx, &g, src) -> MasterRef  a new data graphic master; the callout masters come from
//                                   the document or from `src` (the app's stencil)
//   update(tx, eng, m, &g, src)      the master written anew, every shape that uses it again
//   rename / duplicate / delete      the gallery's menu (delete: from every shape first)
//   applied(doc, shape) / shapes_with(doc, m) / callouts(doc, shape)
//   apply(tx, eng, shapes, m, layer) the graphic on shapes (an earlier one goes first)
//   remove(tx, eng, shapes)          the graphic off again, the old colours back
//   fields(doc, shapes)              the shape data fields the shapes have (label, row name)
//   resolve_field(doc, shape, expr)  "{Label}" -> the formula on the shape's Prop row
//   icon_formula(&rules) / shade_formula(..)   the compiled formulas of icon rules and colours
//   legend::*                        (ST-08, RD-07) the legend: a list of containers of lists
//
// How Visio keeps a data graphic (Visio 2007 developer article "Building Custom Data Graphics",
// the Visio Insights blog, ShapeSheets of files Visio saved; JD19 research):
// * The graphic is a hidden master: its page sheet has User.visUSEType = 1, visDGVisible,
//   visDGShowBorder, visDGHideText, visDGNextItemID, msvDGCalloutGap and the layer
//   "Data Graphic"; its shape is a group with User.visDGDefaultPos = PNT(h,v) (the default
//   position); every item is a subshape with User.msvCalloutType ("Text Callout", "Data Bar",
//   "Icon Set", "Heading"), visDGDefaultPos (TRUE: the item takes the default position),
//   visDGStackHeight, visDGCalloutItem, visDGField0 (the field expression), visDGItemID,
//   visDGUseMaster = USE("<callout master>"), visDGCalloutField = User.visDGField0, and its
//   options in Prop.msvCalloutProp* rows (Min, Max, Label, ValueFormat, ShowValue ...).
// * Applied to a shape, the shape becomes a group and gets one instance of each item's callout
//   master as a subshape: Prop.msvCalloutField's value is a formula on the shape's own Prop row
//   (Sheet.5!Prop.Tasks, SortKey = the field's label), User.visDGDefaultPos follows the shape's
//   control point Controls.msvDGPosition (seven snap points across, BOUND), the pin is a formula
//   of the position and ThePage!User.msvDGCalloutGap, the callout lies on the layer "Data
//   Graphic".  The shape itself gets User.visDGDisplayFormat = USE("{UniqueID of the master}"),
//   visDGDefaultPos, msvLayoutIncludeSubshapes and Controls.msvDGPosition.
// * Positions: PNT(h, v), h 0..6 = far left, left edge, left, centre, right, right edge, far
//   right; v 0..6 = below, bottom edge, bottom, middle, top, top edge, above.  The instance
//   rounds the control's place to -1..5 (= h - 1, "x * 4 / Width").
// * Rendering comes from the callout masters' own cells: a data bar's fill width is a geometry
//   formula of Prop.msvCalloutField and its Min/Max, an icon set shows the geometry section
//   whose NoShow is false for User.msvCalloutIconNumber (-1: none), a text callout shows a text
//   field with the value in Prop.msvCalloutPropValueFormat.
//
// Assumptions where nothing public says how Visio does it ([NF] in the research; read back by
// JFDiagrammer, kept by Visio as ordinary cells):
// * Icon rules: the instance gets Visio's one compiled formula User.msvCalloutIconNumber
//   (IFERROR(IF(cond0,0,IF(cond1,1,...-1)),-1)); the rules themselves stay in the master item's
//   User.JFD_IconRules ("op|a|b" per icon).  A Visio item without them keeps its own
//   msvCalloutIconNumber formula as read (Item::icon_formula) and passes it on.
// * Colour by value: the item is a subshape of the master group with msvCalloutType "Color By
//   Value" (no callout master); its method and colours stay in User.JFD_CBVMethod (0 each
//   value, 1 ranges) and User.JFD_CBVColors ("value|to|#fill|#text" per colour).  On a shape:
//   User.visDGCBVFill = IF(match, RGB(..), ..., the old fill formula), FillForegnd =
//   User.visDGCBVFill; a text colour goes through User.JFD_DGCBVText into Char.Color.
//   User.visDGOldColors keeps what the shape had (our own text: "cell=I" inherited, "cell=F:..."
//   an own formula, "cell=V:..." an own constant, for FillForegnd, Char.Color, HideText and the
//   kind "Kind=Shape") and puts it back when the graphic goes.
// * An item's own position: User.JFD_DGPos = PNT(h,v) with visDGDefaultPos FALSE; a Visio item
//   without it gets the position its pin in the group is nearest to.
// * An item of a master written here is an instance of its callout master (Visio writes a copy
//   of the callout master's shape); instances on shapes are instances as in Visio.
// * visDGUseMaster and visDGDisplayFormat: USE() is not computed by the engine, so the value
//   written with it (254, as seen in Visio's files) stays.
// * visDGCalloutItem on an instance = the item's ID; visDGStackHeight = the distance to the
//   item's place (items at one position stack downwards, above the shape upwards).
// * The helper cells User.JFD_DGH and User.JFD_DGV (the position 0..6 the instance's pin
//   formulas read) are ours: Visio's own pin formulas are not known; ours give the same places.
// * Shape-Text ausblenden: HideText = TRUE on the shape (the old value kept as above); the
//   border (visDGShowBorder) is kept and written but not drawn.
//
// Every edit runs inside the caller's transaction (one undo step) and computes what it wrote
// (Engine::recalc of the sheets it touched).

pub mod legend;
#[cfg(test)]
mod tests;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::cell::{Formula, Value};
use crate::document::{Document, MasterRef, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::instance::{join_fields, split_fields};
use crate::names;
use crate::shape::ShapeKind;
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::units::Unit;

/// the universal name of the layer data graphics lie on
pub const LAYER_U: &str = "Data Graphic";
/// the gap between a shape and the callouts outside it (inches, on paper; Visio's 1/16 in)
pub const GAP: f64 = 0.0625;
/// the value Visio stores with a USE() formula
const USE_VALUE: f64 = 254.0;

/// the kind of an item (Visio's GraphicItem.Type codes)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 2: one icon of a set chosen by rules
    IconSet,
    /// 3: the value as text
    Text,
    /// 4: a bar, stars, a thermometer ... between a minimum and a maximum
    DataBar,
    /// 5: the shape's fill by its value
    ColorByValue,
    /// 6: a heading (Visio's; read and kept, not offered)
    Heading,
}

impl Kind {
    /// Visio's GraphicItem.Type
    pub fn code(self) -> u32 {
        match self {
            Kind::IconSet => 2,
            Kind::Text => 3,
            Kind::DataBar => 4,
            Kind::ColorByValue => 5,
            Kind::Heading => 6,
        }
    }

    /// User.msvCalloutType as Visio writes it
    pub fn callout_type(self) -> &'static str {
        match self {
            Kind::IconSet => "Icon Set",
            Kind::Text => "Text Callout",
            Kind::DataBar => "Data Bar",
            Kind::ColorByValue => "Color By Value",
            Kind::Heading => "Heading",
        }
    }

    /// the kind of a msvCalloutType text (case and spaces ignored)
    pub fn parse(t: &str) -> Option<Kind> {
        let k: String = t.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        match k.as_str() {
            "iconset" => Some(Kind::IconSet),
            "textcallout" | "text" => Some(Kind::Text),
            "databar" => Some(Kind::DataBar),
            "colorbyvalue" | "colourbyvalue" => Some(Kind::ColorByValue),
            "heading" => Some(Kind::Heading),
            _ => None,
        }
    }
}

/// a position at a shape: h 0..6 far left .. far right, v 0..6 below .. above
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Pos {
    pub h: u8,
    pub v: u8,
}

impl Pos {
    /// far right, middle (Visio's default for new graphics)
    pub const DEFAULT: Pos = Pos { h: H_FAR_RIGHT, v: V_MIDDLE };

    /// PNT(h,v)
    pub fn pnt(self) -> String {
        format!("PNT({},{})", self.h, self.v)
    }

    /// the position of a "PNT(h,v)" text (numbers clamped to 0..6)
    pub fn parse(t: &str) -> Option<Pos> {
        let t = t.trim();
        t.get(..4).filter(|p| p.eq_ignore_ascii_case("PNT("))?;
        let body = t[4..].strip_suffix(')')?;
        let (a, b) = body.split_once(',')?;
        let n = |s: &str| s.trim().parse::<f64>().ok().map(|x| crate::formula::math::round(x).clamp(0.0, 6.0) as u8);
        Some(Pos { h: n(a)?, v: n(b)? })
    }
}

/// horizontal positions
pub const H_FAR_LEFT: u8 = 0;
pub const H_LEFT_EDGE: u8 = 1;
pub const H_LEFT: u8 = 2;
pub const H_CENTER: u8 = 3;
pub const H_RIGHT: u8 = 4;
pub const H_RIGHT_EDGE: u8 = 5;
pub const H_FAR_RIGHT: u8 = 6;
/// vertical positions
pub const V_BELOW: u8 = 0;
pub const V_BOTTOM_EDGE: u8 = 1;
pub const V_BOTTOM: u8 = 2;
pub const V_MIDDLE: u8 = 3;
pub const V_TOP: u8 = 4;
pub const V_TOP_EDGE: u8 = 5;
pub const V_ABOVE: u8 = 6;

/// an option of an item: the value of a Prop.msvCalloutProp* row
#[derive(Clone, PartialEq, Debug)]
pub enum Opt {
    Num(f64),
    Text(String),
    Bool(bool),
}

impl Opt {
    /// as a number (a Boolean 1 or 0, a text parsed)
    pub fn num(&self) -> Option<f64> {
        match self {
            Opt::Num(x) => Some(*x),
            Opt::Bool(b) => Some(*b as u8 as f64),
            Opt::Text(t) => parse_num(t),
        }
    }

    /// as text
    pub fn text(&self) -> String {
        match self {
            Opt::Num(x) => crate::formula::fmt_num(*x),
            Opt::Bool(b) => String::from(if *b { "TRUE" } else { "FALSE" }),
            Opt::Text(t) => t.clone(),
        }
    }
}

/// a rule's test of a value
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Op {
    /// [nicht verwendet]: the icon is not used
    NotUsed,
    Equal,
    NotEqual,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    /// between a and b (both included)
    Between,
    Contains,
    NotContains,
}

impl Op {
    /// every test in the dialog's order
    pub const ALL: [Op; 10] = [Op::NotUsed, Op::Equal, Op::NotEqual, Op::Greater, Op::GreaterEq, Op::Less, Op::LessEq, Op::Between, Op::Contains, Op::NotContains];

    /// the code in User.JFD_IconRules
    pub fn code(self) -> &'static str {
        match self {
            Op::NotUsed => "",
            Op::Equal => "=",
            Op::NotEqual => "<>",
            Op::Greater => ">",
            Op::GreaterEq => ">=",
            Op::Less => "<",
            Op::LessEq => "<=",
            Op::Between => "between",
            Op::Contains => "contains",
            Op::NotContains => "notcontains",
        }
    }

    pub fn parse(t: &str) -> Op {
        Op::ALL.iter().copied().find(|o| o.code().eq_ignore_ascii_case(t.trim())).unwrap_or(Op::NotUsed)
    }
}

/// an icon's rule: shown when the value passes the test
#[derive(Clone, PartialEq, Debug)]
pub struct Rule {
    pub op: Op,
    pub a: String,
    /// the upper bound of Between
    pub b: String,
}

impl Rule {
    pub fn new(op: Op, a: &str, b: &str) -> Rule {
        Rule { op, a: String::from(a), b: String::from(b) }
    }
}

/// a colour of a colour-by-value item: a value (or a range from `value` to `to`) and its fill
/// and text colour (0xRRGGBB)
#[derive(Clone, PartialEq, Debug)]
pub struct Shade {
    pub value: String,
    /// the end of a range ("" for each value)
    pub to: String,
    pub fill: u32,
    pub text: Option<u32>,
}

/// an item of a data graphic
#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    /// visDGItemID
    pub id: u32,
    pub kind: Kind,
    /// visDGField0: "{Label}" or a formula of such fields ("{Revenue}-{Expenses}")
    pub field: String,
    /// the NameU of the callout master ("" for a colour by value)
    pub callout: String,
    /// None: the graphic's default position
    pub pos: Option<Pos>,
    /// Prop.msvCalloutProp* values: (row name, value)
    pub options: Vec<(String, Opt)>,
    /// icon sets: the rule of each icon (index = icon number)
    pub rules: Vec<Rule>,
    /// colour by value: ranges (else each value)
    pub ranges: bool,
    pub shades: Vec<Shade>,
    /// an icon number formula read from a file whose rules are not known (kept)
    pub icon_formula: Option<String>,
}

impl Item {
    /// a new item of `kind` for field `field` ("{Label}") shown by callout master `callout`
    pub fn new(kind: Kind, field: &str, callout: &str) -> Item {
        Item {
            id: 0,
            kind,
            field: String::from(field),
            callout: String::from(callout),
            pos: None,
            options: Vec::new(),
            rules: Vec::new(),
            ranges: false,
            shades: Vec::new(),
            icon_formula: None,
        }
    }

    /// an option's value
    pub fn opt(&self, name: &str) -> Option<&Opt> {
        self.options.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v)
    }

    /// set an option
    pub fn set_opt(&mut self, name: &str, v: Opt) {
        match self.options.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(slot) => slot.1 = v,
            None => self.options.push((String::from(name), v)),
        }
    }

    /// the label of a field expression that is one field ("{Kosten}" -> "Kosten")
    pub fn label(&self) -> Option<&str> {
        let t = self.field.trim();
        let inner = t.strip_prefix('{')?.strip_suffix('}')?;
        (!inner.contains(['{', '}'])).then_some(inner)
    }
}

/// a data graphic
#[derive(Clone, PartialEq, Debug)]
pub struct DataGraphic {
    /// the master's name
    pub name: String,
    pub items: Vec<Item>,
    pub default_pos: Pos,
    /// Shape-Text ausblenden (visDGHideText)
    pub hide_text: bool,
    /// Rahmen um Elemente an der Standardposition (visDGShowBorder)
    pub show_border: bool,
    /// visDGNextItemID
    pub next_item: u32,
}

impl DataGraphic {
    pub fn new(name: &str) -> DataGraphic {
        DataGraphic { name: String::from(name), items: Vec::new(), default_pos: Pos::DEFAULT, hide_text: false, show_border: false, next_item: 1 }
    }

    /// add an item (its ID handed out) -> its index
    pub fn push(&mut self, mut item: Item) -> usize {
        let next = self.next_item.max(self.items.iter().map(|i| i.id + 1).max().unwrap_or(1)).max(1);
        item.id = next;
        self.next_item = next + 1;
        self.items.push(item);
        self.items.len() - 1
    }

    /// the item of a field expression
    pub fn item_of(&self, field: &str) -> Option<usize> {
        self.items.iter().position(|i| i.field.trim().eq_ignore_ascii_case(field.trim()))
    }
}

// ---------------------------------------------------------------- small helpers

/// a number typed by a user or kept as text ("1,5" and "1.5" alike)
pub fn parse_num(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().or_else(|| t.replace(',', ".").parse::<f64>().ok()).filter(|x| x.is_finite())
}

fn num_text(x: f64) -> String {
    crate::formula::fmt_num(x)
}

// a string literal of a formula
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// "#rrggbb" of 0xRRGGBB
pub fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

/// 0xRRGGBB of "#rrggbb"
pub fn parse_hex(t: &str) -> Option<u32> {
    let h = t.trim().strip_prefix('#')?;
    (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten()
}

fn rgb(c: u32) -> String {
    format!("RGB({},{},{})", (c >> 16) & 255, (c >> 8) & 255, c & 255)
}

fn user_row<'d>(doc: &'d Document, sheet: SheetRef, name: &str) -> Option<crate::RowView<'d>> {
    doc.user_cells(sheet).into_iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

/// a User cell's value as text (a number as written)
pub fn user_text(doc: &Document, sheet: SheetRef, name: &str) -> Option<String> {
    let r = user_row(doc, sheet, name)?;
    match r.get("Value").value() {
        Value::Text(t) => Some(String::from(doc.str(t))),
        Value::Num(x) => Some(num_text(x)),
        Value::Absent => None,
    }
}

fn user_num(doc: &Document, sheet: SheetRef, name: &str) -> Option<f64> {
    let r = user_row(doc, sheet, name)?;
    match r.get("Value").value() {
        Value::Num(x) => Some(x),
        Value::Text(t) => {
            let t = doc.str(t).trim();
            if t.eq_ignore_ascii_case("TRUE") {
                Some(1.0)
            } else if t.eq_ignore_ascii_case("FALSE") {
                Some(0.0)
            } else {
                parse_num(t)
            }
        }
        Value::Absent => None,
    }
}

fn user_formula(doc: &Document, sheet: SheetRef, name: &str) -> Option<String> {
    let r = user_row(doc, sheet, name)?;
    match r.get("Value").formula() {
        Formula::Expr(f) => Some(String::from(doc.str(f))),
        _ => None,
    }
}

// the string inside USE("...")
fn use_arg(f: &str) -> Option<String> {
    let t = f.trim();
    let head = t.get(..4)?;
    if !head.eq_ignore_ascii_case("USE(") {
        return None;
    }
    let body = t[4..].trim().strip_suffix(')')?.trim();
    let inner = body.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\"\"", "\""))
}

fn single(tx: &mut Transaction, name: &str) -> CellAddr {
    tx.single(name)
}

fn user_addr(tx: &mut Transaction, name: &str) -> CellAddr {
    tx.named("User", name, "Value")
}

fn set_user_str(tx: &mut Transaction, sheet: SheetRef, name: &str, value: &str) -> Result<(), EditError> {
    let v = Value::Text(tx.intern(value));
    tx.set_user(sheet, name, v, Some(Unit::Str))
}

fn set_user_bool(tx: &mut Transaction, sheet: SheetRef, name: &str, value: bool) -> Result<(), EditError> {
    tx.set_user(sheet, name, Value::Num(value as u8 as f64), Some(Unit::Bool))
}

fn set_user_num(tx: &mut Transaction, sheet: SheetRef, name: &str, value: f64) -> Result<(), EditError> {
    tx.set_user(sheet, name, Value::Num(value), None)
}

// a USE() formula with the value Visio stores with it
fn set_user_use(tx: &mut Transaction, sheet: SheetRef, name: &str, arg: &str) -> Result<(), EditError> {
    tx.set_user_formula(sheet, name, &format!("USE({})", quoted(arg)))?;
    let a = user_addr(tx, name);
    tx.set_result(sheet, &a, Value::Num(USE_VALUE), None)
}

// a formula into a cell, forced past a GUARD
fn force(tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, formula: &str) -> Result<(), EditError> {
    tx.force_formula(sheet, addr, formula)
}

// a named row's cell, the row added when the sheet lacks it
fn named_cell(tx: &mut Transaction, sheet: SheetRef, section: crate::strings::Sym, row: &str, cell: &str) -> Result<CellAddr, EditError> {
    let (sk, rk) = tx.ensure_named_row(sheet, section, row)?;
    let c = tx.intern(cell);
    Ok(CellAddr::row(sk, rk, c))
}

/// a name no master of the document has (`base`, then "base (2)" ...)
pub fn unique_name(doc: &Document, base: &str) -> String {
    let taken = |n: &str| {
        doc.masters().any(|m| {
            doc.master(m).is_some_and(|x| x.header.name.or(x.header.name_u).is_some_and(|s| doc.str(s).eq_ignore_ascii_case(n)) || x.header.name_u.is_some_and(|s| doc.str(s).eq_ignore_ascii_case(n)))
        })
    };
    let base = base.trim();
    if !base.is_empty() && !taken(base) {
        return String::from(base);
    }
    (2..10_000).map(|k| format!("{base} ({k})")).find(|c| !taken(c)).unwrap_or_else(|| String::from(base))
}

// a name-based GUID ("{XXXXXXXX-XXXX-4XXX-8XXX-XXXXXXXXXXXX}") of the parts: the same parts give
// the same ID (files written twice are the same)
fn guid(parts: &[&str]) -> String {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x9e37_79b9_7f4a_7c15;
    for p in parts {
        for byte in p.bytes().chain(core::iter::once(0xff)) {
            a = (a ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
            b = (b ^ a).rotate_left(23).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
    }
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let (hi, lo) = (mix(a ^ b.rotate_left(7)), mix(b ^ a.rotate_left(13)));
    let d1 = (hi >> 32) as u32;
    let d2 = (hi >> 16) as u16;
    let d3 = ((hi as u16) & 0x0fff) | 0x4000;
    let d4 = (((lo >> 48) as u16) & 0x3fff) | 0x8000;
    let d5 = lo & 0xffff_ffff_ffff;
    format!("{{{d1:08X}-{d2:04X}-{d3:04X}-{d4:04X}-{d5:012X}}}")
}

// ---------------------------------------------------------------- reading

/// whether master m is a data graphic (User.visUSEType = 1 in its page sheet)
pub fn is_graphic(doc: &Document, m: MasterRef) -> bool {
    doc.master(m).is_some() && user_num(doc, SheetRef::Master(m), "visUSEType") == Some(1.0)
}

/// the document's data graphics in master order
pub fn graphics(doc: &Document) -> Vec<MasterRef> {
    doc.masters().filter(|&m| is_graphic(doc, m)).collect()
}

/// the name of a master (Name, else NameU)
pub fn master_name(doc: &Document, m: MasterRef) -> String {
    doc.master(m).and_then(|x| x.header.name.or(x.header.name_u)).map(|s| String::from(doc.str(s))).unwrap_or_default()
}

fn master_group(doc: &Document, m: MasterRef) -> Option<ShapeRef> {
    let slot = doc.master(m)?.master_shape()?;
    Some(ShapeRef { owner: Owner::Master(m), slot })
}

/// the model of data graphic master m (None: not a data graphic)
pub fn read(doc: &Document, m: MasterRef) -> Option<DataGraphic> {
    if !is_graphic(doc, m) {
        return None;
    }
    let ps = SheetRef::Master(m);
    let mut g = DataGraphic::new(&master_name(doc, m));
    g.hide_text = user_num(doc, ps, "visDGHideText").is_some_and(|x| x != 0.0);
    g.show_border = user_num(doc, ps, "visDGShowBorder").is_some_and(|x| x != 0.0);
    g.next_item = user_num(doc, ps, "visDGNextItemID").map_or(1, |x| x.max(1.0) as u32);
    let Some(group) = master_group(doc, m) else { return Some(g) };
    let gs = SheetRef::Shape(group);
    g.default_pos = user_formula(doc, gs, "visDGDefaultPos").and_then(|f| Pos::parse(&f)).unwrap_or(Pos::DEFAULT);
    let kids: Vec<ShapeRef> = doc.shape(group).map(|s| s.children().collect()).unwrap_or_default();
    for k in kids {
        if let Some(item) = read_item(doc, group, k) {
            g.items.push(item);
        }
    }
    g.next_item = g.next_item.max(g.items.iter().map(|i| i.id + 1).max().unwrap_or(1));
    Some(g)
}

fn read_item(doc: &Document, group: ShapeRef, k: ShapeRef) -> Option<Item> {
    let ks = SheetRef::Shape(k);
    let kind = Kind::parse(&user_text(doc, ks, "msvCalloutType")?)?;
    let field = user_text(doc, ks, "visDGField0").unwrap_or_default();
    let callout = user_formula(doc, ks, "visDGUseMaster")
        .and_then(|f| use_arg(&f))
        .or_else(|| doc.master_of(k).and_then(|cm| doc.master(cm)).and_then(|x| x.header.name_u.or(x.header.name)).map(|s| String::from(doc.str(s))))
        .unwrap_or_default();
    let mut item = Item::new(kind, &field, if kind == Kind::ColorByValue { "" } else { &callout });
    item.id = user_num(doc, ks, "visDGItemID").map_or(0, |x| x.max(0.0) as u32);
    let uses_default = user_num(doc, ks, "visDGDefaultPos").is_none_or(|x| x != 0.0);
    item.pos = if uses_default { None } else { user_formula(doc, ks, "JFD_DGPos").or_else(|| user_text(doc, ks, "JFD_DGPos")).and_then(|t| Pos::parse(&t)).or_else(|| pos_of_pin(doc, group, k)) };
    for r in doc.shape_data(ks) {
        let Some(name) = r.name() else { continue };
        if !name.starts_with("msvCalloutProp") {
            continue;
        }
        let c = r.get("Value");
        let v = match (c.value(), c.unit()) {
            (Value::Num(x), Some(Unit::Bool)) => Opt::Bool(x != 0.0),
            (Value::Num(x), _) => Opt::Num(x),
            (Value::Text(t), _) => {
                let s = doc.str(t);
                if s.eq_ignore_ascii_case("TRUE") || s.eq_ignore_ascii_case("FALSE") {
                    Opt::Bool(s.eq_ignore_ascii_case("TRUE"))
                } else {
                    Opt::Text(String::from(s))
                }
            }
            (Value::Absent, _) => continue,
        };
        item.options.push((String::from(name), v));
    }
    if let Some(t) = user_text(doc, ks, "JFD_IconRules") {
        item.rules = split_fields(&t)
            .iter()
            .map(|r| {
                let f = split_fields(r);
                let get = |i: usize| f.get(i).cloned().unwrap_or_default();
                Rule { op: Op::parse(&get(0)), a: get(1), b: get(2) }
            })
            .collect();
    } else if kind == Kind::IconSet {
        // a graphic of Visio's: its compiled formula is all there is
        let local = doc.sheet(ks).and_then(|s| {
            let u = doc.sym("User")?;
            let n = doc.sym("msvCalloutIconNumber")?;
            let v = doc.sym("Value")?;
            match s.find(&CellAddr::row(SecKey::one(u), RowKey::Named(n), v)) {
                crate::sheet::Local::Cell(c) => c.formula.expr(),
                _ => None,
            }
        });
        item.icon_formula = local.map(|f| String::from(doc.str(f)));
    }
    if kind == Kind::ColorByValue {
        item.ranges = user_num(doc, ks, "JFD_CBVMethod").is_some_and(|x| x != 0.0);
        if let Some(t) = user_text(doc, ks, "JFD_CBVColors") {
            item.shades = split_fields(&t)
                .iter()
                .filter_map(|e| {
                    let f = split_fields(e);
                    Some(Shade { value: f.first()?.clone(), to: f.get(1).cloned().unwrap_or_default(), fill: parse_hex(f.get(2)?)?, text: f.get(3).and_then(|t| parse_hex(t)) })
                })
                .collect();
        }
    }
    Some(item)
}

// the position an item's pin in the group is nearest to (a Visio item at its own place)
fn pos_of_pin(doc: &Document, group: ShapeRef, k: ShapeRef) -> Option<Pos> {
    let n = |s: ShapeRef, c: crate::strings::Sym| doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).num();
    let (gw, gh) = (n(group, names::WIDTH)?, n(group, names::HEIGHT)?);
    let (x, y, w, h) = (n(k, names::PIN_X)?, n(k, names::PIN_Y)?, n(k, names::WIDTH)?, n(k, names::HEIGHT)?);
    let lx = n(k, names::LOC_PIN_X).unwrap_or(w * 0.5);
    let ly = n(k, names::LOC_PIN_Y).unwrap_or(h * 0.5);
    let near = |left: f64, big: f64, small: f64| -> u8 {
        let c = [-small - GAP, -small * 0.5, 0.0, (big - small) * 0.5, big - small, big - small * 0.5, big + GAP];
        let mut best = 0;
        for i in 1..7 {
            if (c[i] - left).abs() < (c[best] - left).abs() {
                best = i;
            }
        }
        best as u8
    };
    Some(Pos { h: near(x - lx, gw, w), v: near(y - ly, gh, h) })
}

// ---------------------------------------------------------------- pins

// the left (bottom) edge of a callout at position p (0..6) along an axis as a formula: `big`
// the shape's size (Sheet.N!Width), `own` the callout's (Width)
fn edge_formula(sel: &str, big: &str, own: &str) -> String {
    let gap = "ThePage!User.msvDGCalloutGap";
    let e = [
        format!("-{own}-{gap}"),
        format!("-{own}*0.5"),
        String::from("0"),
        format!("({big}-{own})*0.5"),
        format!("{big}-{own}"),
        format!("{big}-{own}*0.5"),
        format!("{big}+{gap}"),
    ];
    let mut f = e[6].clone();
    for k in (0..6).rev() {
        f = format!("IF({sel}<{}.5,{},{f})", k, e[k]);
    }
    f
}

/// the pin formulas of a callout inside group `gid` (its ID): PinX, PinY (stacked by
/// User.visDGStackHeight: down, up above the shape)
pub fn pin_formulas(gid: u32) -> (String, String) {
    let px = format!("{}+LocPinX", edge_formula("User.JFD_DGH", &format!("Sheet.{gid}!Width"), "Width"));
    let py = format!("{}+LocPinY+IF(User.JFD_DGV>5.5,1,-1)*User.visDGStackHeight", edge_formula("User.JFD_DGV", &format!("Sheet.{gid}!Height"), "Height"));
    (px, py)
}

/// the position the instance takes from shape `gid`'s control point (-1..5, Visio's scale)
fn control_k(gid: u32, axis: &str, size: &str) -> String {
    format!("BOUND(ROUND(Sheet.{gid}!Controls.msvDGPosition.{axis}*4/Sheet.{gid}!{size},0),0,FALSE,-1,5)")
}

// the BOUND formula of the control point: seven snap points across `size` (Width / Height)
fn control_formula(size: &str, p: u8) -> String {
    let mut f = format!("BOUND({size}*{},0", num_text((p as f64 - 1.0) * 0.25));
    for k in 0..7 {
        let x = num_text((k as f64 - 1.0) * 0.25);
        f.push_str(&format!(",FALSE,{size}*{x},{size}*{x}"));
    }
    f.push(')');
    f
}

// ---------------------------------------------------------------- fields

/// a shape data field: its label (as shown) and row name
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    pub label: String,
    pub row: String,
}

/// the shape data fields of the shapes, by row name, in the order they are met (the
/// callouts' own msvCallout* rows left out)
pub fn fields(doc: &Document, shapes: &[ShapeRef]) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    for &s in shapes {
        for r in doc.shape_data(SheetRef::Shape(s)) {
            let Some(name) = r.name() else { continue };
            if name.starts_with("msvCallout") || out.iter().any(|f| f.row == name) {
                continue;
            }
            let label = r.text("Label").filter(|l| !l.is_empty()).unwrap_or(name);
            out.push(Field { label: String::from(label), row: String::from(name) });
        }
    }
    out
}

/// "{Label}" of a field
pub fn field_expr(label: &str) -> String {
    format!("{{{label}}}")
}

// the row name of a field label on a shape (label first, then the row name)
fn row_of(doc: &Document, shape: ShapeRef, label: &str) -> Option<String> {
    let rows = doc.shape_data(SheetRef::Shape(shape));
    let by_label = rows.iter().find(|r| r.text("Label").is_some_and(|l| l.trim().eq_ignore_ascii_case(label.trim())));
    let by_name = || rows.iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case(label.trim())));
    by_label.or_else(by_name).and_then(|r| r.name()).map(String::from)
}

/// a field expression on a shape -> (the formula, the label): every "{Label}" becomes the
/// shape's Prop row (`Sheet.N!Prop.Row` with `sheet` Some(N), else `Prop.Row`); None when the
/// shape lacks one of them
pub fn resolve_field(doc: &Document, shape: ShapeRef, expr: &str, sheet: Option<u32>) -> Option<(String, String)> {
    let mut out = String::new();
    let mut rest = expr;
    let mut labels = Vec::new();
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let j = tail.find('}')?;
        let label = &tail[..j];
        let row = row_of(doc, shape, label)?;
        match sheet {
            Some(n) => out.push_str(&format!("Sheet.{n}!Prop.{row}")),
            None => out.push_str(&format!("Prop.{row}")),
        }
        labels.push(label);
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    if labels.is_empty() || out.trim().is_empty() {
        return None;
    }
    let label = if labels.len() == 1 && expr.trim() == field_expr(labels[0]) { String::from(labels[0]) } else { String::from(expr.trim()) };
    Some((out, label))
}

// ---------------------------------------------------------------- compiled formulas

// the test of one rule on value `v` (None: never true)
fn condition(op: Op, a: &str, b: &str, v: &str) -> Option<String> {
    let na = parse_num(a);
    let nb = parse_num(b);
    let cmp = |o: &str| na.map(|x| format!("{v}{o}{}", num_text(x)));
    match op {
        Op::NotUsed => None,
        Op::Equal => Some(match na {
            Some(x) => format!("{v}={}", num_text(x)),
            None => format!("STRSAME({v},{},TRUE)", quoted(a.trim())),
        }),
        Op::NotEqual => Some(match na {
            Some(x) => format!("{v}<>{}", num_text(x)),
            None => format!("NOT(STRSAME({v},{},TRUE))", quoted(a.trim())),
        }),
        Op::Greater => cmp(">"),
        Op::GreaterEq => cmp(">="),
        Op::Less => cmp("<"),
        Op::LessEq => cmp("<="),
        Op::Between => {
            let (x, y) = (na?, nb?);
            Some(format!("AND({v}>={},{v}<={})", num_text(x.min(y)), num_text(x.max(y))))
        }
        Op::Contains => Some(format!("NOT(ISERROR(FIND({},{v},1,TRUE)))", quoted(a))),
        Op::NotContains => Some(format!("ISERROR(FIND({},{v},1,TRUE))", quoted(a))),
    }
}

/// User.msvCalloutIconNumber of icon rules: the number of the first icon whose rule the value
/// passes, -1 for none (errors: -1)
pub fn icon_formula(rules: &[Rule]) -> String {
    let v = "Prop.msvCalloutField";
    let mut f = String::from("-1");
    for (k, r) in rules.iter().enumerate().rev() {
        if let Some(c) = condition(r.op, &r.a, &r.b, v) {
            f = format!("IF({c},{k},{f})");
        }
    }
    format!("IFERROR({f},-1)")
}

/// the colour of a value: IF(match, RGB(..), ...) with `otherwise` when none matches
pub fn shade_formula(shades: &[Shade], ranges: bool, value: &str, text: bool, otherwise: &str) -> String {
    let mut f = String::from(otherwise);
    for s in shades.iter().rev() {
        let c = match (text, s.text) {
            (false, _) => s.fill,
            (true, Some(t)) => t,
            (true, None) => continue,
        };
        let cond = if ranges {
            match (parse_num(&s.value), parse_num(&s.to)) {
                (Some(a), Some(b)) => format!("AND({value}>={},{value}<={})", num_text(a.min(b)), num_text(a.max(b))),
                (Some(a), None) => format!("{value}>={}", num_text(a)),
                _ => condition(Op::Equal, &s.value, "", value).unwrap_or_default(),
            }
        } else {
            condition(Op::Equal, &s.value, "", value).unwrap_or_default()
        };
        if cond.is_empty() {
            continue;
        }
        f = format!("IF({cond},{},{f})", rgb(c));
    }
    format!("IFERROR({f},{otherwise})")
}

// ---------------------------------------------------------------- writing a master

// the callout master of an item in the document: there already, else taken from `src`
fn callout_master(tx: &mut Transaction, name_u: &str, src: Option<&Document>) -> Result<MasterRef, EditError> {
    if let Some(m) = find_master(tx.doc(), name_u) {
        return Ok(m);
    }
    let src = src.ok_or(EditError::NoMaster)?;
    let m = find_master(src, name_u).ok_or(EditError::NoMaster)?;
    crate::instance::import_master(tx, src, m)
}

/// a master by NameU (else Name), ASCII case ignored
pub fn find_master(doc: &Document, name_u: &str) -> Option<MasterRef> {
    let by = |f: fn(&crate::MasterHeader) -> Option<crate::Sym>| doc.masters().find(|&m| doc.master(m).and_then(|x| f(&x.header)).is_some_and(|n| doc.str(n).eq_ignore_ascii_case(name_u)));
    by(|h| h.name_u).or_else(|| by(|h| h.name))
}

// the layer rows of a master's page sheet: "Data Graphic" -> its index
fn master_layer(tx: &mut Transaction, sheet: SheetRef) -> Result<u32, EditError> {
    let sk = SecKey::one(names::LAYER);
    for r in tx.doc().section_rows(sheet, sk) {
        if r.text("NameUniv").or_else(|| r.text("Name")) == Some(LAYER_U) {
            if let RowKey::Index(ix) = r.key() {
                return Ok(ix);
            }
        }
    }
    let RowKey::Index(ix) = tx.add_row(sheet, sk, None, None)? else { return Err(EditError::Invalid) };
    let rk = RowKey::Index(ix);
    let n = tx.intern(LAYER_U);
    tx.set_constant(sheet, &CellAddr::row(sk, rk, names::NAME), Value::Text(n), None)?;
    tx.set_constant(sheet, &CellAddr::row(sk, rk, names::NAME_UNIV), Value::Text(n), None)?;
    for (cell, v) in [(names::VISIBLE, 1.0), (names::PRINT, 1.0), (names::ACTIVE, 0.0), (names::LOCK, 0.0), (names::SNAP, 1.0), (names::GLUE, 1.0), (names::COLOR, 255.0), (names::COLOR_TRANS, 0.0)] {
        tx.set_constant(sheet, &CellAddr::row(sk, rk, cell), Value::Num(v), None)?;
    }
    Ok(ix)
}

// every shape of a subtree
fn subtree(doc: &Document, s: ShapeRef) -> Vec<ShapeRef> {
    let mut out = Vec::new();
    let mut stack = alloc::vec![s];
    while let Some(x) = stack.pop() {
        out.push(x);
        if let Some(sh) = doc.shape(x) {
            stack.extend(sh.children());
        }
    }
    out
}

// the height of a master's shape
fn master_size(doc: &Document, m: MasterRef) -> (f64, f64) {
    let Some(g) = master_group(doc, m) else { return (0.25, 0.25) };
    let n = |c| doc.resolve(SheetRef::Shape(g), &CellAddr::single(c)).num();
    (n(names::WIDTH).unwrap_or(0.25), n(names::HEIGHT).unwrap_or(0.25))
}

// the option values of an item onto a callout (the instance in the master or on a shape):
// only those that differ from the callout master's
fn write_options(tx: &mut Transaction, sheet: SheetRef, item: &Item) -> Result<(), EditError> {
    for (name, v) in &item.options {
        let addr = named_cell(tx, sheet, names::PROPERTY, name, "Value")?;
        let cur = tx.doc().resolve(sheet, &addr);
        let same = match (v, cur.value()) {
            (Opt::Num(x), Value::Num(y)) => (x - y).abs() < 1e-12 && cur.unit() != Some(Unit::Bool),
            (Opt::Bool(b), Value::Num(y)) => cur.unit() == Some(Unit::Bool) && (*b as u8 as f64 - y).abs() < 1e-12,
            (Opt::Text(t), Value::Text(s)) => tx.doc().str(s) == t.as_str(),
            _ => false,
        };
        if same && !matches!(cur.formula(), Formula::Expr(_)) {
            continue;
        }
        match v {
            Opt::Num(x) => tx.force_constant(sheet, &addr, Value::Num(*x), None)?,
            Opt::Bool(b) => tx.force_constant(sheet, &addr, Value::Num(*b as u8 as f64), Some(Unit::Bool))?,
            Opt::Text(t) => {
                let s = tx.intern(t);
                tx.force_constant(sheet, &addr, Value::Text(s), Some(Unit::Str))?
            }
        }
    }
    Ok(())
}

// the cells every callout of an item carries (in the master and on shapes): type, position
// helpers, options, the icon formula, the pin; `follow`: an item at the default position takes
// it from shape `gid`'s control point (on shapes), else the position is fixed (`at`)
fn write_callout_cells(tx: &mut Transaction, sheet: SheetRef, item: &Item, gid: u32, at: Pos, follow: bool, stack: f64) -> Result<(), EditError> {
    set_user_str(tx, sheet, "msvCalloutType", item.kind.callout_type())?;
    let (h, v) = if follow && item.pos.is_none() {
        (format!("{}+1", control_k(gid, "X", "Width")), format!("{}+1", control_k(gid, "Y", "Height")))
    } else {
        (num_text(at.h as f64), num_text(at.v as f64))
    };
    tx.set_user_formula(sheet, "JFD_DGH", &h)?;
    tx.set_user_formula(sheet, "JFD_DGV", &v)?;
    tx.set_user(sheet, "visDGStackHeight", Value::Num(stack), Some(Unit::Inches))?;
    write_options(tx, sheet, item)?;
    if item.kind == Kind::IconSet {
        let f = match (&item.icon_formula, item.rules.is_empty()) {
            (Some(f), true) => f.clone(),
            _ => icon_formula(&item.rules),
        };
        tx.set_user_formula(sheet, "msvCalloutIconNumber", &f)?;
    }
    let (px, py) = pin_formulas(gid);
    let (ax, ay) = (single(tx, "PinX"), single(tx, "PinY"));
    force(tx, sheet, &ax, &px)?;
    force(tx, sheet, &ay, &py)
}

// the master's content: page sheet cells, the group, one subshape per item
fn fill_master(tx: &mut Transaction, m: MasterRef, g: &DataGraphic, src: Option<&Document>) -> Result<Vec<SheetRef>, EditError> {
    let ps = SheetRef::Master(m);
    let owner = Owner::Master(m);
    // the callout masters first (they come into the document)
    let mut callouts: Vec<Option<MasterRef>> = Vec::new();
    for it in &g.items {
        callouts.push(match it.kind {
            Kind::ColorByValue => None,
            _ => Some(callout_master(tx, &it.callout, src)?),
        });
    }
    set_user_num(tx, ps, "visUSEType", 1.0)?;
    set_user_bool(tx, ps, "visDGVisible", true)?;
    set_user_bool(tx, ps, "visDGShowBorder", g.show_border)?;
    set_user_bool(tx, ps, "visDGHideText", g.hide_text)?;
    let next = g.next_item.max(g.items.iter().map(|i| i.id + 1).max().unwrap_or(1));
    set_user_num(tx, ps, "visDGNextItemID", next as f64)?;
    for (cell, v) in [("PageWidth", 1.0), ("PageHeight", 1.0), ("PageScale", 1.0), ("DrawingScale", 1.0)] {
        let a = single(tx, cell);
        if tx.doc().resolve(ps, &a).num().is_none() {
            tx.set_constant(ps, &a, Value::Num(v), Some(Unit::Inches))?;
        }
    }
    tx.set_user_formula(ps, "msvDGCalloutGap", &format!("DrawingScale/PageScale*{}", num_text(GAP)))?;
    let layer = master_layer(tx, ps)?;
    // the group: 1 x 1 in on the master page
    let group = tx.add_shape(owner, None, ShapeKind::Group)?;
    let gs = SheetRef::Shape(group);
    for (cell, v) in [("PinX", 0.5), ("PinY", 0.5), ("Width", 1.0), ("Height", 1.0)] {
        let a = single(tx, cell);
        tx.set_constant(gs, &a, Value::Num(v), Some(Unit::Inches))?;
    }
    for (cell, f) in [("LocPinX", "Width*0.5"), ("LocPinY", "Height*0.5")] {
        let a = single(tx, cell);
        tx.set_formula(gs, &a, f)?;
    }
    tx.set_user_formula(gs, "visDGDefaultPos", &g.default_pos.pnt())?;
    tx.set_layers(group, &[layer])?;
    let gid = tx.doc().shape(group).map_or(0, |s| s.id());
    let mut sheets = alloc::vec![ps, gs];
    let mut stacks: Vec<(Pos, f64)> = Vec::new();
    for (it, cm) in g.items.iter().zip(callouts) {
        let k = match cm {
            Some(cm) => {
                let k = tx.add_instance(owner, cm, 0.0, 0.0)?;
                tx.move_shape(k, Some(group), None)?;
                k
            }
            None => {
                let k = tx.add_shape(owner, Some(group), ShapeKind::Shape)?;
                let ks = SheetRef::Shape(k);
                for (cell, v) in [("Width", 0.25), ("Height", 0.25)] {
                    let a = single(tx, cell);
                    tx.set_constant(ks, &a, Value::Num(v), Some(Unit::Inches))?;
                }
                for (cell, f) in [("LocPinX", "Width*0.5"), ("LocPinY", "Height*0.5")] {
                    let a = single(tx, cell);
                    tx.set_formula(ks, &a, f)?;
                }
                k
            }
        };
        let ks = SheetRef::Shape(k);
        let eff = it.pos.unwrap_or(g.default_pos);
        let h = cm.map_or(0.25, |cm| master_size(tx.doc(), cm).1);
        let stack = stack_at(&mut stacks, eff, h);
        write_callout_cells(tx, ks, it, gid, eff, false, stack)?;
        set_user_bool(tx, ks, "visDGDefaultPos", it.pos.is_none())?;
        if let Some(p) = it.pos {
            tx.set_user_formula(ks, "JFD_DGPos", &p.pnt())?;
        }
        set_user_num(tx, ks, "visDGCalloutItem", 1.0)?;
        set_user_str(tx, ks, "visDGField0", &it.field)?;
        set_user_num(tx, ks, "visDGItemID", it.id as f64)?;
        if it.kind != Kind::ColorByValue {
            set_user_use(tx, ks, "visDGUseMaster", &it.callout)?;
        }
        tx.set_user_formula(ks, "visDGCalloutField", "User.visDGField0")?;
        if !it.rules.is_empty() {
            let rules: Vec<String> = it.rules.iter().map(|r| join_fields(&[r.op.code(), &r.a, &r.b])).collect();
            let refs: Vec<&str> = rules.iter().map(|s| s.as_str()).collect();
            set_user_str(tx, ks, "JFD_IconRules", &join_fields(&refs))?;
        }
        if it.kind == Kind::ColorByValue {
            set_user_num(tx, ks, "JFD_CBVMethod", it.ranges as u8 as f64)?;
            let shades: Vec<String> = it.shades.iter().map(|s| join_fields(&[&s.value, &s.to, &hex(s.fill), &s.text.map(hex).unwrap_or_default()])).collect();
            let refs: Vec<&str> = shades.iter().map(|s| s.as_str()).collect();
            set_user_str(tx, ks, "JFD_CBVColors", &join_fields(&refs))?;
        }
        for x in subtree(tx.doc(), k) {
            tx.set_layers(x, &[layer])?;
            sheets.push(SheetRef::Shape(x));
        }
    }
    Ok(sheets)
}

// the distance of an item from its place: what earlier items at the same place took
fn stack_at(stacks: &mut Vec<(Pos, f64)>, at: Pos, h: f64) -> f64 {
    const SPACE: f64 = 0.02;
    match stacks.iter_mut().find(|(p, _)| *p == at) {
        Some(slot) => {
            let s = slot.1;
            slot.1 += h + SPACE;
            s
        }
        None => {
            stacks.push((at, h + SPACE));
            0.0
        }
    }
}

fn set_master_attrs(tx: &mut Transaction, m: MasterRef, uid: &str) -> Result<(), EditError> {
    let mut h = tx.doc().master(m).ok_or(EditError::NoMaster)?.header.clone();
    let hidden = (tx.intern("Hidden"), tx.intern("1"));
    if let Some(slot) = h.attrs.iter_mut().find(|(n, _)| *n == hidden.0) {
        slot.1 = hidden.1;
    } else {
        h.attrs.push(hidden);
    }
    if h.unique_id.is_none() {
        h.unique_id = Some(tx.intern(uid));
    }
    if h.base_id.is_none() {
        h.base_id = Some(tx.intern(uid));
    }
    tx.set_master_header(m, h)
}

/// a new data graphic master (hidden) for `g`; the callout masters its items name come from
/// the document or from `src` -> the master.  The caller recalculates nothing: the master's
/// cells are computed here.
pub fn create(tx: &mut Transaction, eng: &mut Engine, g: &DataGraphic, src: Option<&Document>) -> Result<MasterRef, EditError> {
    tx.atomic(|tx| {
        let name = unique_name(tx.doc(), &g.name);
        let m = tx.add_master(&name, &name)?;
        let id = tx.doc().master(m).map_or(0, |x| x.header.id);
        let uid = guid(&["JFDiagrammer data graphic", &name, &format!("{id}"), &format!("{}", tx.doc().masters().count())]);
        set_master_attrs(tx, m, &uid)?;
        let mut g = g.clone();
        g.name = name;
        let sheets = fill_master(tx, m, &g, src)?;
        eng.recalc(tx, &sheets)?;
        Ok(m)
    })
}

/// write data graphic master m anew from `g` (its name, items, positions, options) and put
/// it again on every shape that uses it -> those shapes
pub fn update(tx: &mut Transaction, eng: &mut Engine, m: MasterRef, g: &DataGraphic, src: Option<&Document>, layer_name: &str) -> Result<Vec<ShapeRef>, EditError> {
    if !is_graphic(tx.doc(), m) {
        return Err(EditError::NoMaster);
    }
    tx.atomic(|tx| {
        let users = shapes_with(tx.doc(), m);
        remove(tx, eng, &users)?;
        let tops: Vec<ShapeRef> = tx.doc().top_shapes(Owner::Master(m)).collect();
        tx.delete_shapes(&tops)?;
        if master_name(tx.doc(), m) != g.name && !g.name.trim().is_empty() {
            rename(tx, m, &g.name)?;
        }
        let sheets = fill_master(tx, m, g, src)?;
        eng.recalc(tx, &sheets)?;
        apply(tx, eng, &users, m, layer_name)
    })
}

/// rename a data graphic (Name and NameU; shapes keep it by its UniqueID)
pub fn rename(tx: &mut Transaction, m: MasterRef, name: &str) -> Result<(), EditError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(EditError::Invalid);
    }
    let taken = find_master(tx.doc(), name).is_some_and(|x| x != m);
    if taken {
        return Err(EditError::Exists);
    }
    tx.rename_master(m, name, name)
}

/// a copy of data graphic m under a new name ("name (2)" when taken) -> the copy
pub fn duplicate(tx: &mut Transaction, eng: &mut Engine, m: MasterRef, name: &str) -> Result<MasterRef, EditError> {
    let mut g = read(tx.doc(), m).ok_or(EditError::NoMaster)?;
    g.name = String::from(name);
    create(tx, eng, &g, None)
}

/// delete data graphic m: off every shape first, then the master (its callout masters stay)
pub fn delete(tx: &mut Transaction, eng: &mut Engine, m: MasterRef) -> Result<(), EditError> {
    if !is_graphic(tx.doc(), m) {
        return Err(EditError::NoMaster);
    }
    tx.atomic(|tx| {
        let users = shapes_with(tx.doc(), m);
        remove(tx, eng, &users)?;
        tx.delete_master(m)
    })
}

// ---------------------------------------------------------------- applied

/// the data graphic a shape shows (User.visDGDisplayFormat = USE("{UniqueID}" or name))
pub fn applied(doc: &Document, shape: ShapeRef) -> Option<MasterRef> {
    let f = user_formula(doc, SheetRef::Shape(shape), "visDGDisplayFormat")?;
    let key = use_arg(&f)?;
    let by_uid = doc.masters().find(|&m| doc.master(m).and_then(|x| x.header.unique_id).is_some_and(|u| doc.str(u).trim_matches(['{', '}']).eq_ignore_ascii_case(key.trim_matches(['{', '}']))));
    by_uid.or_else(|| find_master(doc, &key)).filter(|&m| is_graphic(doc, m))
}

/// every shape of the document's pages that shows data graphic m
pub fn shapes_with(doc: &Document, m: MasterRef) -> Vec<ShapeRef> {
    let mut out = Vec::new();
    for p in doc.pages() {
        for s in doc.all_shapes(Owner::Page(p)) {
            if applied(doc, s) == Some(m) {
                out.push(s);
            }
        }
    }
    out
}

/// the data graphic callouts of a shape (subshapes with msvCalloutType and visDGCalloutItem)
pub fn callouts(doc: &Document, shape: ShapeRef) -> Vec<ShapeRef> {
    let Some(sh) = doc.shape(shape) else { return Vec::new() };
    sh.children()
        .filter(|&k| {
            let ks = SheetRef::Shape(k);
            user_row(doc, ks, "msvCalloutType").is_some() && user_row(doc, ks, "visDGCalloutItem").is_some()
        })
        .collect()
}

/// the page's layer for data graphics (NameUniv "Data Graphic"), added with `name` when the
/// page has none -> its index
pub fn page_layer(tx: &mut Transaction, page: PageRef, name: &str) -> Result<u32, EditError> {
    if let Some(ix) = tx.doc().layer_by_name(page, LAYER_U) {
        return Ok(ix);
    }
    let n = if name.trim().is_empty() { LAYER_U } else { name };
    tx.add_layer(page, n, LAYER_U)
}

// ---------------------------------------------------------------- old colours

const OLD_CELLS: [&str; 3] = ["FillForegnd", "Char.Color", "HideText"];

fn old_addr(tx: &mut Transaction, cell: &str) -> CellAddr {
    match cell {
        "Char.Color" => {
            let c = tx.intern("Color");
            CellAddr::row(SecKey::one(names::CHARACTER), RowKey::Index(0), c)
        }
        other => tx.single(other),
    }
}

// what the shape has in a cell: "I" inherited, "F:<formula>" an own formula, "V:<value>"
fn old_state(tx: &mut Transaction, sheet: SheetRef, cell: &str) -> String {
    let addr = old_addr(tx, cell);
    let doc = tx.doc();
    let Some(sh) = doc.sheet(sheet) else { return String::from("I") };
    match sh.find(&addr) {
        crate::sheet::Local::Cell(c) => match c.formula {
            Formula::Expr(f) => format!("F:{}", doc.str(f)),
            _ => match c.value {
                Value::Num(x) => format!("V:{}", num_text(x)),
                Value::Text(t) => format!("V:{}", doc.str(t)),
                Value::Absent => String::from("I"),
            },
        },
        _ => String::from("I"),
    }
}

// the formula that gives what a cell shows now (its own or inherited formula, else its value)
fn current_formula(doc: &Document, sheet: SheetRef, addr: &CellAddr, fallback: &str) -> String {
    let r = doc.resolve(sheet, addr);
    match r.formula() {
        Formula::Expr(f) => String::from(doc.str(f)),
        _ => match r.value() {
            Value::Num(x) => num_text(x),
            Value::Text(t) => {
                let s = doc.str(t);
                match parse_hex(s) {
                    Some(c) => rgb(c),
                    None => String::from(fallback),
                }
            }
            Value::Absent => String::from(fallback),
        },
    }
}

fn restore(tx: &mut Transaction, sheet: SheetRef, cell: &str, state: &str) -> Result<(), EditError> {
    let addr = old_addr(tx, cell);
    if state == "I" || state.is_empty() {
        return tx.inherit_cell(sheet, &addr);
    }
    if let Some(f) = state.strip_prefix("F:") {
        return tx.force_formula(sheet, &addr, f);
    }
    if let Some(v) = state.strip_prefix("V:") {
        let value = match parse_num(v) {
            Some(x) if !v.starts_with('#') => Value::Num(x),
            _ => Value::Text(tx.intern(v)),
        };
        return tx.force_constant(sheet, &addr, value, None);
    }
    Ok(())
}

// a row the shape got from the graphic goes: an own row is removed, cells over an inherited
// one are taken back
fn drop_row(tx: &mut Transaction, sheet: SheetRef, section: crate::strings::Sym, name: &str) -> Result<(), EditError> {
    let sk = SecKey::one(section);
    let rk = RowKey::Named(tx.intern(name));
    let Some(sh) = tx.doc().sheet(sheet) else { return Ok(()) };
    let own = sh.section(sk).and_then(|s| s.row(rk)).map(|r| r.cells.iter().map(|c| c.name).collect::<Vec<_>>());
    let Some(cells) = own else { return Ok(()) };
    let cat = crate::names::Category::of_section(tx.doc().str(section));
    let inherited = tx.doc().parent_sheet(sheet, cat).is_some_and(|p| tx.doc().has_row(p, sk, rk));
    if inherited {
        for c in cells {
            tx.inherit_cell(sheet, &CellAddr::row(sk, rk, c))?;
        }
        Ok(())
    } else {
        tx.delete_row(sheet, sk, rk)
    }
}

// ---------------------------------------------------------------- apply and remove

/// put data graphic m on `shapes` (an earlier graphic goes first; shapes on pages only) ->
/// the shapes that show it now (a shape none of whose fields the graphic names is left out)
pub fn apply(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], m: MasterRef, layer_name: &str) -> Result<Vec<ShapeRef>, EditError> {
    let g = read(tx.doc(), m).ok_or(EditError::NoMaster)?;
    let key = tx.doc().master(m).and_then(|x| x.header.unique_id.or(x.header.name_u)).map(|s| String::from(tx.doc().str(s))).ok_or(EditError::NoMaster)?;
    tx.atomic(|tx| {
        let mut done = Vec::new();
        let mut sheets: Vec<SheetRef> = Vec::new();
        for &s in shapes {
            let Owner::Page(page) = s.owner else { continue };
            if tx.doc().shape(s).is_none() || callouts_parent(tx.doc(), s) {
                continue;
            }
            if applied(tx.doc(), s).is_some() {
                remove(tx, eng, &[s])?;
            }
            if apply_one(tx, s, page, &g, &key, layer_name, &mut sheets)? {
                done.push(s);
            }
        }
        if !sheets.is_empty() {
            eng.recalc(tx, &sheets)?;
        }
        Ok(done)
    })
}

// a callout of a data graphic itself (never a target)
fn callouts_parent(doc: &Document, s: ShapeRef) -> bool {
    user_row(doc, SheetRef::Shape(s), "visDGCalloutItem").is_some()
}

fn apply_one(tx: &mut Transaction, s: ShapeRef, page: PageRef, g: &DataGraphic, key: &str, layer_name: &str, sheets: &mut Vec<SheetRef>) -> Result<bool, EditError> {
    let sid = tx.doc().shape(s).map_or(0, |x| x.id());
    // the items whose fields the shape has
    let mut plan: Vec<(&Item, String, String)> = Vec::new();
    for it in &g.items {
        let local = it.kind == Kind::ColorByValue;
        if let Some((f, label)) = resolve_field(tx.doc(), s, &it.field, if local { None } else { Some(sid) }) {
            if local || find_master(tx.doc(), &it.callout).is_some() {
                plan.push((it, f, label));
            }
        }
    }
    if plan.is_empty() {
        return Ok(false);
    }
    let sheet = SheetRef::Shape(s);
    let was_shape = tx.doc().shape(s).is_some_and(|x| x.header().kind != ShapeKind::Group);
    // what the graphic changes, kept for its removal
    let mut old: Vec<String> = Vec::new();
    for c in OLD_CELLS {
        let st = old_state(tx, sheet, c);
        old.push(format!("{c}={st}"));
    }
    if was_shape {
        old.push(String::from("Kind=Shape"));
    }
    let refs: Vec<&str> = old.iter().map(|x| x.as_str()).collect();
    set_user_str(tx, sheet, "visDGOldColors", &join_fields(&refs))?;
    set_user_use(tx, sheet, "visDGDisplayFormat", key)?;
    tx.set_user_formula(sheet, "visDGDefaultPos", &g.default_pos.pnt())?;
    tx.set_user_formula(sheet, "msvLayoutIncludeSubshapes", "OR(TRUE,DEPENDSON(FALSE))")?;
    // the control point that moves the graphic
    let cx = named_cell(tx, sheet, names::CONTROL, "msvDGPosition", "X")?;
    force(tx, sheet, &cx, &control_formula("Width", g.default_pos.h))?;
    let cy = named_cell(tx, sheet, names::CONTROL, "msvDGPosition", "Y")?;
    force(tx, sheet, &cy, &control_formula("Height", g.default_pos.v))?;
    for (cell, f) in [("XDyn", "Controls.msvDGPosition"), ("YDyn", "Controls.msvDGPosition.Y")] {
        let a = named_cell(tx, sheet, names::CONTROL, "msvDGPosition", cell)?;
        tx.force_formula(sheet, &a, f)?;
    }
    for (cell, v) in [("XCon", 0.0), ("YCon", 0.0), ("CanGlue", 0.0)] {
        let a = named_cell(tx, sheet, names::CONTROL, "msvDGPosition", cell)?;
        tx.force_constant(sheet, &a, Value::Num(v), if cell == "CanGlue" { Some(Unit::Bool) } else { None })?;
    }
    if g.hide_text {
        let a = tx.single("HideText");
        tx.force_constant(sheet, &a, Value::Num(1.0), Some(Unit::Bool))?;
    }
    // the gap on the page
    let ps = SheetRef::Page(page);
    if user_row(tx.doc(), ps, "msvDGCalloutGap").is_none() {
        tx.set_user_formula(ps, "msvDGCalloutGap", &format!("DrawingScale/PageScale*{}", num_text(GAP)))?;
        sheets.push(ps);
    }
    let layer = page_layer(tx, page, layer_name)?;
    let mut stacks: Vec<(Pos, f64)> = Vec::new();
    for (it, f, label) in plan {
        if it.kind == Kind::ColorByValue {
            let fill_addr = tx.single("FillForegnd");
            let otherwise = current_formula(tx.doc(), sheet, &fill_addr, "RGB(255,255,255)");
            let ff = shade_formula(&it.shades, it.ranges, &format!("({f})"), false, &otherwise);
            tx.set_user_formula(sheet, "visDGCBVFill", &ff)?;
            force(tx, sheet, &fill_addr, "User.visDGCBVFill")?;
            if it.shades.iter().any(|x| x.text.is_some()) {
                let caddr = old_addr(tx, "Char.Color");
                let otherwise = current_formula(tx.doc(), sheet, &caddr, "0");
                let tf = shade_formula(&it.shades, it.ranges, &format!("({f})"), true, &otherwise);
                tx.set_user_formula(sheet, "JFD_DGCBVText", &tf)?;
                // a shape without its own character row: the text colour stays (Annahme)
                let _ = force(tx, sheet, &caddr, "User.JFD_DGCBVText");
            }
            continue;
        }
        let cm = find_master(tx.doc(), &it.callout).ok_or(EditError::NoMaster)?;
        let k = tx.add_instance(Owner::Page(page), cm, 0.0, 0.0)?;
        tx.move_shape(k, Some(s), None)?;
        let ks = SheetRef::Shape(k);
        let eff = it.pos.unwrap_or(g.default_pos);
        let h = master_size(tx.doc(), cm).1;
        let stack = stack_at(&mut stacks, eff, h);
        let fa = named_cell(tx, ks, names::PROPERTY, "msvCalloutField", "Value")?;
        force(tx, ks, &fa, &f)?;
        let sk = named_cell(tx, ks, names::PROPERTY, "msvCalloutField", "SortKey")?;
        let lab = tx.intern(&label);
        tx.force_constant(ks, &sk, Value::Text(lab), Some(Unit::Str))?;
        write_callout_cells(tx, ks, it, sid, eff, true, stack)?;
        tx.set_user_formula(
            ks,
            "visDGDefaultPos",
            &format!("PNT({},{})", control_k(sid, "X", "Width"), control_k(sid, "Y", "Height")),
        )?;
        set_user_num(tx, ks, "visDGCalloutItem", it.id as f64)?;
        for x in subtree(tx.doc(), k) {
            tx.set_layers(x, &[layer])?;
            sheets.push(SheetRef::Shape(x));
        }
    }
    sheets.push(sheet);
    Ok(true)
}

/// take the data graphic off `shapes`: its callouts go, the shape's colours, text and kind
/// come back, the graphic's rows go
pub fn remove(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef]) -> Result<(), EditError> {
    tx.atomic(|tx| {
        let mut sheets = Vec::new();
        for &s in shapes {
            let doc = tx.doc();
            if doc.shape(s).is_none() || (user_row(doc, SheetRef::Shape(s), "visDGDisplayFormat").is_none() && callouts(doc, s).is_empty()) {
                continue;
            }
            remove_one(tx, s)?;
            sheets.push(SheetRef::Shape(s));
        }
        if !sheets.is_empty() {
            eng.recalc(tx, &sheets)?;
        }
        Ok(())
    })
}

fn remove_one(tx: &mut Transaction, s: ShapeRef) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    let kids = callouts(tx.doc(), s);
    if !kids.is_empty() {
        tx.delete_shapes(&kids)?;
    }
    let old = user_text(tx.doc(), sheet, "visDGOldColors").unwrap_or_default();
    let mut back_to_shape = false;
    for entry in split_fields(&old) {
        let Some((cell, state)) = entry.split_once('=') else { continue };
        match cell {
            "Kind" => back_to_shape = state == "Shape",
            c if OLD_CELLS.contains(&c) => restore(tx, sheet, c, state)?,
            _ => {}
        }
    }
    for name in ["visDGDisplayFormat", "visDGDefaultPos", "msvLayoutIncludeSubshapes", "visDGCBVFill", "visDGOldColors", "JFD_DGCBVText"] {
        drop_row(tx, sheet, names::USER, name)?;
    }
    drop_row(tx, sheet, names::CONTROL, "msvDGPosition")?;
    if back_to_shape && tx.doc().shape(s).is_some_and(|x| x.children().next().is_none()) {
        let mut h = tx.doc().shape(s).ok_or(EditError::NoShape)?.header().clone();
        h.kind = ShapeKind::Shape;
        tx.set_shape_header(s, h)?;
    }
    Ok(())
}

/// the text of a graphic's items for lists: "Kosten (Text)" ... (the caller's words)
pub fn describe(item: &Item, kinds: &[&str; 4]) -> String {
    let k = match item.kind {
        Kind::Text | Kind::Heading => kinds[0],
        Kind::DataBar => kinds[1],
        Kind::IconSet => kinds[2],
        Kind::ColorByValue => kinds[3],
    };
    format!("{} ({k})", item.label().map_or_else(|| item.field.clone(), |l| l.to_string()))
}
