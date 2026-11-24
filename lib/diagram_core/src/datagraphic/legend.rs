// legend -- (JD19, ST-08, RD-07) the legend of the data graphics on a page
//
//   Texts                       the words the legend writes (the caller's language): its title,
//                               the tests of icon rules, the line of a text item
//   groups(doc, page, &texts)   what the legend shows: one group per item of the graphics the
//                               page's shapes show (its field and kind once), each with its
//                               entries: an icon and its rule, a colour and its value or range,
//                               a bar and its range, a text item
//   insert(tx, eng, page, horizontal, &texts) -> ShapeRef   the legend as Visio 2010 and later
//                               build it: an outer list ("Legende", horizontal or vertical), one
//                               container per group with the field as its heading, inside an
//                               invisible list of the entries; at the top right of the page
//
// The legend is built with diagram_core::structure (Relationships on both sides, the lists
// laid out, the containers fitted) from plain shapes, so it is edited like any list: entries can
// be retyped and dragged.  Like Visio's it is not kept up to date: after the graphics change
// the user inserts it again.  Its outer list carries User.JFD_DGLegend = TRUE (own cell; how
// Visio marks its legend is not known).  The swatches are instances of the items' callout
// masters (an icon with its number fixed, a bar half full) and small rectangles in the colours.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::ops::{self, Basic};
use crate::structure;

/// the words of a legend in the caller's language
#[derive(Clone, Debug)]
pub struct Texts {
    /// the outer list's heading ("Legende")
    pub title: String,
    /// the entry of a text item ("Wert als Text")
    pub text: String,
    /// the tests in the order of Op::ALL ("", "=", "≠", ">" ... "zwischen", "enthält" ...)
    pub ops: [String; 10],
    /// an icon whose rule is not known ("Symbol {}")
    pub icon: String,
}

impl Default for Texts {
    fn default() -> Texts {
        let ops = ["", "=", "<>", ">", ">=", "<", "<=", "between", "contains", "does not contain"].map(String::from);
        Texts { title: String::from("Legend"), text: String::from("Value as text"), ops, icon: String::from("Icon {}") }
    }
}

/// the picture of an entry
#[derive(Clone, PartialEq, Debug)]
pub enum Swatch {
    /// an instance of a callout master: an icon number, a value for the field
    Callout { master: MasterRef, icon: Option<i32>, value: Option<f64> },
    /// a rectangle in this colour (0xRRGGBB)
    Color(u32),
    /// text only
    None,
}

/// an entry: its picture and text
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub swatch: Swatch,
    pub text: String,
}

/// a group: the field's label and the entries
#[derive(Clone, PartialEq, Debug)]
pub struct Group {
    pub label: String,
    pub entries: Vec<Entry>,
}

fn rule_text(r: &Rule, t: &Texts) -> String {
    let word = t.ops.get(Op::ALL.iter().position(|&o| o == r.op).unwrap_or(0)).cloned().unwrap_or_default();
    match r.op {
        Op::Between => format!("{} – {}", r.a.trim(), r.b.trim()),
        Op::Contains | Op::NotContains => format!("{word} „{}“", r.a.trim()),
        _ => format!("{word} {}", r.a.trim()),
    }
}

fn shade_text(s: &Shade, ranges: bool) -> String {
    if ranges && !s.to.trim().is_empty() {
        format!("{} – {}", s.value.trim(), s.to.trim())
    } else {
        String::from(s.value.trim())
    }
}

/// the groups of the legend of the graphics shown on `page`
pub fn groups(doc: &Document, page: PageRef, t: &Texts) -> Vec<Group> {
    let mut seen: Vec<MasterRef> = Vec::new();
    for s in doc.all_shapes(Owner::Page(page)) {
        if let Some(m) = applied(doc, s) {
            if !seen.contains(&m) {
                seen.push(m);
            }
        }
    }
    let mut out: Vec<Group> = Vec::new();
    let mut keys: Vec<(String, Kind)> = Vec::new();
    for m in seen {
        let Some(g) = read(doc, m) else { continue };
        for it in &g.items {
            let label = it.label().map_or_else(|| it.field.clone(), String::from);
            let key = (label.to_lowercase(), it.kind);
            if keys.contains(&key) {
                continue;
            }
            let cm = find_master(doc, &it.callout);
            let mut entries = Vec::new();
            match it.kind {
                Kind::Text | Kind::Heading => entries.push(Entry { swatch: Swatch::None, text: t.text.clone() }),
                Kind::DataBar => {
                    let (lo, hi) = (it.opt("msvCalloutPropMin").and_then(|o| o.num()).unwrap_or(0.0), it.opt("msvCalloutPropMax").and_then(|o| o.num()).unwrap_or(100.0));
                    let swatch = cm.map_or(Swatch::None, |m| Swatch::Callout { master: m, icon: None, value: Some((lo + hi) * 0.5) });
                    entries.push(Entry { swatch, text: format!("{} – {}", num_text(lo), num_text(hi)) });
                }
                Kind::IconSet => {
                    if it.rules.is_empty() {
                        // a graphic of Visio's: its icons without their rules
                        let n = cm.and_then(|m| master_group(doc, m)).and_then(|g| user_num(doc, SheetRef::Shape(g), "msvCalloutIconCount")).unwrap_or(0.0).clamp(0.0, 5.0) as i32;
                        for k in 0..n {
                            entries.push(Entry { swatch: cm.map_or(Swatch::None, |m| Swatch::Callout { master: m, icon: Some(k), value: None }), text: t.icon.replace("{}", &format!("{}", k + 1)) });
                        }
                    }
                    for (k, r) in it.rules.iter().enumerate() {
                        if r.op == Op::NotUsed {
                            continue;
                        }
                        entries.push(Entry { swatch: cm.map_or(Swatch::None, |m| Swatch::Callout { master: m, icon: Some(k as i32), value: None }), text: rule_text(r, t) });
                    }
                }
                Kind::ColorByValue => {
                    for s in &it.shades {
                        entries.push(Entry { swatch: Swatch::Color(s.fill), text: shade_text(s, it.ranges) });
                    }
                }
            }
            if entries.is_empty() {
                continue;
            }
            keys.push(key);
            out.push(Group { label, entries });
        }
    }
    out
}

// sizes in inches (on paper; the page's drawing scale is not applied: legends sit on 1:1 pages)
const ROW_H: f64 = 0.25;
const HEAD: f64 = 0.28;
const MARGIN: f64 = 0.05;
const SPACE: f64 = 0.03;
const CHAR_W: f64 = 0.085;

fn set_f(tx: &mut Transaction, sheet: SheetRef, cell: &str, f: &str) -> Result<(), EditError> {
    let a = tx.single(cell);
    tx.force_formula(sheet, &a, f)
}

// the first geometry section's NoShow (invisible lists, text entries)
fn hide_geometry(tx: &mut Transaction, s: ShapeRef) -> Result<(), EditError> {
    let sk = SecKey { name: names::GEOMETRY, index: 0 };
    tx.force_constant(SheetRef::Shape(s), &CellAddr::section(sk, names::NO_SHOW), Value::Num(1.0), Some(Unit::Bool))
}

// a box with a heading on top (Container or List): text block and line in the heading
fn with_heading(tx: &mut Transaction, s: ShapeRef, text: &str) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    set_user_str(tx, sheet, "JFD_HeadingEdge", "Top")?;
    tx.set_user(sheet, "JFD_HeadingSize", Value::Num(HEAD), Some(Unit::Inches))?;
    for (c, f) in [("TxtWidth", "Width"), ("TxtHeight", "User.JFD_HeadingSize"), ("TxtPinX", "Width*0.5"), ("TxtPinY", "Height-User.JFD_HeadingSize*0.5"), ("TxtLocPinX", "TxtWidth*0.5"), ("TxtLocPinY", "TxtHeight*0.5")] {
        set_f(tx, sheet, c, f)?;
    }
    let sk = tx.add_section(sheet, "Geometry")?;
    tx.force_constant(sheet, &CellAddr::section(sk, names::NO_FILL), Value::Num(1.0), Some(Unit::Bool))?;
    for (i, kind) in ["MoveTo", "LineTo"].into_iter().enumerate() {
        let r = tx.add_row(sheet, sk, None, Some(kind))?;
        let x = if i == 0 { "0" } else { "Width" };
        tx.force_formula(sheet, &CellAddr::row(sk, r, names::X), x)?;
        tx.force_formula(sheet, &CellAddr::row(sk, r, names::Y), "Height-User.JFD_HeadingSize")?;
    }
    if !text.is_empty() {
        tx.set_text(s, Some(crate::shape::Text::plain(text)))?;
    }
    Ok(())
}

fn structure_cells(tx: &mut Transaction, s: ShapeRef, kind: &str, direction: Option<i64>, margin: f64) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    set_user_str(tx, sheet, "msvStructureType", kind)?;
    tx.set_user(sheet, "msvSDContainerMargin", Value::Num(margin), Some(Unit::Inches))?;
    tx.set_user(sheet, "msvSDContainerResize", Value::Num(2.0), None)?;
    if let Some(d) = direction {
        tx.set_user(sheet, "msvSDListDirection", Value::Num(d as f64), None)?;
        tx.set_user(sheet, "msvSDListSpacing", Value::Num(SPACE), Some(Unit::Inches))?;
        tx.set_user(sheet, "msvSDListAlignment", Value::Num(0.0), None)?;
    }
    Ok(())
}

// an entry: a text box (no line, no fill) with its swatch as a subshape on the left
fn entry(tx: &mut Transaction, eng: &mut Engine, page: PageRef, at: [f64; 2], e: &Entry, layer: u32) -> Result<ShapeRef, EditError> {
    let owner = Owner::Page(page);
    // the swatch's size
    let (sw, sh) = match &e.swatch {
        Swatch::Callout { master, .. } => master_size(tx.doc(), *master),
        Swatch::Color(_) => (0.3, 0.18),
        Swatch::None => (0.0, 0.0),
    };
    let tw = (e.text.chars().count() as f64 * CHAR_W).max(0.4);
    let lead = if sw > 0.0 { sw + 0.08 } else { 0.0 };
    let (w, h) = (lead + tw + 0.1, ROW_H.max(sh + 0.04));
    let s = ops::new_shape(tx, eng, owner, Basic::Rectangle, at, [at[0] + w, at[1] + h])?;
    let sheet = SheetRef::Shape(s);
    hide_geometry(tx, s)?;
    tx.set_text(s, Some(crate::shape::Text::plain(&e.text)))?;
    for (c, f) in [("TxtWidth", format!("Width-{}", num_text(lead))), ("TxtHeight", String::from("Height")), ("TxtPinX", format!("{}+TxtWidth*0.5", num_text(lead))), ("TxtPinY", String::from("Height*0.5")), ("TxtLocPinX", String::from("TxtWidth*0.5")), ("TxtLocPinY", String::from("TxtHeight*0.5"))] {
        set_f(tx, sheet, c, &f)?;
    }
    // left aligned (a document without a paragraph row in its styles keeps the centre)
    let ha = tx.intern("HorzAlign");
    let pa = CellAddr::row(SecKey::one(names::PARAGRAPH), RowKey::Index(0), ha);
    if tx.force_constant(sheet, &pa, Value::Num(0.0), None).is_err() && tx.add_row(sheet, SecKey::one(names::PARAGRAPH), Some(RowKey::Index(0)), None).is_ok() {
        let _ = tx.force_constant(sheet, &pa, Value::Num(0.0), None);
    }
    let sub = match &e.swatch {
        Swatch::None => None,
        Swatch::Color(c) => {
            let k = ops::new_shape(tx, eng, owner, Basic::Rectangle, [0.0, 0.0], [sw, sh])?;
            let ks = SheetRef::Shape(k);
            let fa = tx.single("FillForegnd");
            tx.force_formula(ks, &fa, &rgb(*c))?;
            let fp = tx.single("FillPattern");
            tx.force_constant(ks, &fp, Value::Num(1.0), None)?;
            Some(k)
        }
        Swatch::Callout { master, icon, value } => {
            let k = tx.add_instance(owner, *master, 0.0, 0.0)?;
            let ks = SheetRef::Shape(k);
            if let Some(v) = value {
                let a = named_cell(tx, ks, names::PROPERTY, "msvCalloutField", "Value")?;
                tx.force_constant(ks, &a, Value::Num(*v), None)?;
                // the bar alone, without its number
                let sv = named_cell(tx, ks, names::PROPERTY, "msvCalloutPropShowValue", "Value")?;
                tx.force_constant(ks, &sv, Value::Num(0.0), Some(Unit::Bool))?;
            }
            if let Some(n) = icon {
                let a = user_addr(tx, "msvCalloutIconNumber");
                if tx.doc().has_row(ks, SecKey::one(names::USER), a.row.unwrap_or(RowKey::Index(0))) {
                    tx.force_constant(ks, &a, Value::Num(*n as f64), None)?;
                } else {
                    set_user_num(tx, ks, "msvCalloutIconNumber", *n as f64)?;
                }
            }
            Some(k)
        }
    };
    if let Some(k) = sub {
        tx.move_shape(k, Some(s), None)?;
        let ks = SheetRef::Shape(k);
        set_f(tx, ks, "PinX", &format!("LocPinX+{}", num_text(0.04)))?;
        set_f(tx, ks, "PinY", &format!("Sheet.{}!Height*0.5-Height*0.5+LocPinY", tx.doc().shape(s).map_or(0, |x| x.id())))?;
        for x in subtree(tx.doc(), k) {
            tx.set_layers(x, &[layer])?;
        }
        let sheets: Vec<SheetRef> = subtree(tx.doc(), k).into_iter().map(SheetRef::Shape).collect();
        eng.recalc(tx, &sheets)?;
    }
    tx.set_layers(s, &[layer])?;
    eng.recalc(tx, &[sheet])?;
    Ok(s)
}

/// the legend of the data graphics on `page` (see the file header) at the top right of the
/// page -> its outer list (Invalid: the page shows no data graphic)
pub fn insert(tx: &mut Transaction, eng: &mut Engine, page: PageRef, horizontal: bool, t: &Texts, layer_name: &str) -> Result<ShapeRef, EditError> {
    let gs = groups(tx.doc(), page, t);
    if gs.is_empty() {
        return Err(EditError::Invalid);
    }
    tx.atomic(|tx| {
        let owner = Owner::Page(page);
        let layer = page_layer(tx, page, layer_name)?;
        let mut boxes = Vec::new();
        for g in &gs {
            // the entries in an invisible list
            let mut es = Vec::new();
            for e in &g.entries {
                es.push(entry(tx, eng, page, [0.0, 0.0], e, layer)?);
            }
            let inner = ops::new_shape(tx, eng, owner, Basic::Rectangle, [0.0, 0.0], [1.0, 1.0])?;
            hide_geometry(tx, inner)?;
            structure_cells(tx, inner, "List", Some(2), 0.0)?;
            structure::add_members(tx, eng, inner, &es, None)?;
            structure::layout_list(tx, eng, inner)?;
            // the container with the field as its heading
            let b = structure::shape_box(tx.doc(), inner);
            let c = ops::new_shape(tx, eng, owner, Basic::Rectangle, [b[0] - MARGIN, b[1] - MARGIN], [b[2] + MARGIN, b[3] + MARGIN + HEAD])?;
            structure_cells(tx, c, "Container", None, MARGIN)?;
            with_heading(tx, c, &g.label)?;
            eng.recalc(tx, &[SheetRef::Shape(c)])?;
            structure::add_members(tx, eng, c, &[inner], None)?;
            structure::fit(tx, eng, c, true)?;
            tx.bring_to_front(inner)?;
            for &e in &es {
                tx.bring_to_front(e)?;
            }
            for x in [inner, c] {
                tx.set_layers(x, &[layer])?;
            }
            boxes.push(c);
        }
        // the outer list
        let outer = ops::new_shape(tx, eng, owner, Basic::Rectangle, [0.0, 0.0], [1.0, 1.0])?;
        structure_cells(tx, outer, "List", Some(if horizontal { 0 } else { 2 }), MARGIN)?;
        set_user_bool(tx, SheetRef::Shape(outer), "JFD_DGLegend", true)?;
        // the containers as wide as the widest
        set_user_bool(tx, SheetRef::Shape(outer), "JFD_ListStretch", true)?;
        with_heading(tx, outer, &t.title)?;
        tx.set_layers(outer, &[layer])?;
        eng.recalc(tx, &[SheetRef::Shape(outer)])?;
        tx.send_to_back(outer)?;
        structure::add_members(tx, eng, outer, &boxes, None)?;
        structure::layout_list(tx, eng, outer)?;
        // to the top right corner of the page
        let ps = SheetRef::Page(page);
        let (pwa, pha) = (tx.single("PageWidth"), tx.single("PageHeight"));
        let pw = tx.doc().resolve(ps, &pwa).num().unwrap_or(8.5);
        let ph = tx.doc().resolve(ps, &pha).num().unwrap_or(11.0);
        let b = structure::shape_box(tx.doc(), outer);
        let d = [pw - 0.25 - b[2], ph - 0.25 - b[3]];
        let along = structure::carried(tx.doc(), &[outer]);
        ops::move_shapes(tx, eng, &along, d)?;
        Ok(outer)
    })
}

/// whether a shape is (the outer list of) a data graphic legend
pub fn is_legend(doc: &Document, s: ShapeRef) -> bool {
    user_num(doc, SheetRef::Shape(s), "JFD_DGLegend").is_some_and(|x| x != 0.0)
}
