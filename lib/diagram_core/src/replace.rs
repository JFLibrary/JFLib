// replace -- (JD21) Shape ändern (SP-11, RS-28, SH-G05): shapes replaced by instances of a master,
// keeping what the user gave them -- place, size, text, shape data, format, glue, membership
//
//   Behavior::of(doc, shape)            the Change Shape Behavior cells a replacement reads
//   can_replace(doc, shape, master)     whether `shape` can become an instance of `master`
//   replace(tx, eng, shapes, master, lang) -> Vec<ShapeRef>   every shape that can, replaced; the
//                                       new shapes in the order given
//   class(doc, sheet)                   User.msvReplaceClass (the gallery offers the same class)
//
// Visio's rules (belegt: learn.microsoft.com "Change Shape Behavior Section", ReplaceLockFormat,
// ReplaceLockText, ReplaceLockShapeData, ReplaceCopyCells; Visio blog "Insights for developers
// about Change Shape" (2012); support.microsoft.com "Replace one shape with another"):
// * It works like deleting the old shape and dropping the new one (new ID, the master's name);
//   "position, connections, formatting, shape text, and shape data" stay.  Only 2-D for 2-D and
//   1-D for 1-D; a container only by a container (the gallery offers containers only); list
//   shapes (a flowchart's lanes) are never replaced; LockReplace, PageLockReplace and
//   DocLockReplace refuse.  User.msvReplaceClass narrows the gallery to masters of the same class.
// * What moves over are the old shape's LOCAL values: shape data, text, format, and the cells of
//   Hyperlink, Layer, Protection, Alignment, Layout, 1-D Endpoints and Shape Transform.  What it
//   inherits from its master does not.  Actions, User cells, Controls, Geometry never move.
// * The cells of the NEW master decide: ReplaceLockShapeData TRUE keeps the master's shape data,
//   ReplaceLockText TRUE the master's text (and its Character, Paragraph, Tabs, Text Fields and
//   Text Block Format), ReplaceLockFormat TRUE the master's format (Fill, Line, Quick Style, Theme,
//   Gradient, Bevel, Effects, gradient stops).  ReplaceCopyCells = DEPENDSON(cell, ...) names
//   cells that are copied whatever the locks say (formula and value; value only where the new
//   shape lacks a cell the formula names).
// Angenommen (the Gegenprobe checks): connectors glued to a connection point keep it when the new
// shape has a point with that index, else they glue to the whole shape (dynamic) or let go (a
// plain line); the new shape takes the old one's place in the z-order and in its group, its
// container and list memberships, callouts and boundary edges (the Relationships of both sides);
// a container keeps its members; the text's Character/Paragraph/Tabs/Field rows come with the
// text; Text Transform does not move; a group's own styles move when they differ from its
// master's (or it has no master); comments (kept as a raw part, HL-04) and data links (JD18)
// stay with the old ID -- not yet carried over.

use alloc::vec::Vec;

use crate::cell::{Cell, Formula, Value};
use crate::document::{Document, MasterRef, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::glue::{self, End, Target};
use crate::names::{self, Category};
use crate::ops::text;
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::structure::{self, rel};

#[cfg(test)]
mod tests;

/// the cells of the Change Shape Behavior section (read from the new shape's master)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Behavior {
    /// ReplaceLockShapeData: the master's shape data stays
    pub lock_data: bool,
    /// ReplaceLockText: the master's text stays
    pub lock_text: bool,
    /// ReplaceLockFormat: the master's format stays
    pub lock_format: bool,
    /// ReplaceCopyCells: the cells copied anyway (names as the formula writes them)
    pub copy: Vec<alloc::string::String>,
}

fn flag(doc: &Document, sheet: SheetRef, cell: &str) -> bool {
    doc.sym(cell).and_then(|c| doc.resolve(sheet, &CellAddr::single(c)).num()).is_some_and(|v| v != 0.0)
}

impl Behavior {
    /// the behaviour cells of a shape sheet (with inheritance)
    pub fn of(doc: &Document, sheet: SheetRef) -> Behavior {
        let copy = doc
            .sym("ReplaceCopyCells")
            .and_then(|c| match doc.resolve(sheet, &CellAddr::single(c)).formula() {
                Formula::Expr(f) => Some(alloc::string::String::from(doc.str(f))),
                _ => None,
            })
            .map(|f| {
                text::items(&f)
                    .into_iter()
                    .filter_map(|it| match it {
                        text::Item::Cell { name, sheet: None, foreign: false, .. } => Some(alloc::string::String::from(name)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Behavior { lock_data: flag(doc, sheet, "ReplaceLockShapeData"), lock_text: flag(doc, sheet, "ReplaceLockText"), lock_format: flag(doc, sheet, "ReplaceLockFormat"), copy }
    }
}

/// User.msvReplaceClass of a sheet (None: none)
pub fn class(doc: &Document, sheet: SheetRef) -> Option<alloc::string::String> {
    let r = doc.user_cells(sheet).into_iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case("msvReplaceClass")))?;
    match r.get("Value").value() {
        Value::Text(t) => {
            let t = doc.str(t).trim();
            (!t.is_empty()).then(|| alloc::string::String::from(t))
        }
        _ => None,
    }
}

fn master_sheet(doc: &Document, m: MasterRef) -> Option<SheetRef> {
    let slot = doc.master(m)?.master_shape()?;
    Some(SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot }))
}

// the master is 1-D (its shape has BeginX)
fn master_one_d(doc: &Document, m: MasterRef) -> bool {
    master_sheet(doc, m).is_some_and(|s| doc.resolve(s, &CellAddr::single(names::BEGIN_X)).exists())
}

fn structure_kind(doc: &Document, sheet: SheetRef) -> Option<alloc::string::String> {
    let r = doc.user_cells(sheet).into_iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case("msvStructureType")))?;
    match r.get("Value").value() {
        Value::Text(t) => Some(doc.str(t).trim().to_ascii_lowercase()),
        _ => None,
    }
}

/// whether `shape` can be replaced by an instance of `master` (a master of the same document):
/// 1-D for 1-D, 2-D for 2-D, a container only by a container, no list, no guide, no
/// LockReplace / PageLockReplace / DocLockReplace
pub fn can_replace(doc: &Document, shape: ShapeRef, master: MasterRef) -> bool {
    let Some(sh) = doc.shape(shape) else { return false };
    if sh.is_deleted() || sh.header().kind == crate::shape::ShapeKind::Guide || doc.master(master).is_none() {
        return false;
    }
    // an inheriting subshape of an instance stays its master's
    if sh.header().master_shape.is_some() && sh.header().master.is_none() {
        return false;
    }
    let sheet = SheetRef::Shape(shape);
    let page = match shape.owner {
        Owner::Page(p) => SheetRef::Page(p),
        Owner::Master(m) => SheetRef::Master(m),
    };
    if flag(doc, sheet, "LockReplace") || flag(doc, page, "PageLockReplace") || flag(doc, SheetRef::Document, "DocLockReplace") {
        return false;
    }
    if crate::ops::one_d(doc, shape) != master_one_d(doc, master) {
        return false;
    }
    let old = structure_kind(doc, sheet);
    let new = master_sheet(doc, master).and_then(|m| structure_kind(doc, m));
    if old.as_deref() == Some("list") || new.as_deref() == Some("list") {
        return false;
    }
    (old.as_deref() == Some("container")) == (new.as_deref() == Some("container"))
}

// the single cells a replacement takes over (local ones): Shape Transform, 1-D Endpoints,
// Protection, Alignment, Shape Layout, Miscellaneous' layer cell
const TRANSFORM: &[&str] = &["Width", "Height", "PinX", "PinY", "LocPinX", "LocPinY", "Angle", "FlipX", "FlipY", "ResizeMode"];
const ENDS: &[&str] = &["BeginX", "BeginY", "EndX", "EndY"];
const ALIGN: &[&str] = &["AlignLeft", "AlignCenter", "AlignRight", "AlignTop", "AlignMiddle", "AlignBottom"];
const LAYOUT: &[&str] = &[
    "ShapePermeableX", "ShapePermeableY", "ShapePermeablePlace", "ShapeFixedCode", "ShapePlowCode", "ShapeRouteStyle", "ShapePlaceFlip",
    "ShapePlaceStyle", "ShapeSplit", "ShapeSplittable", "ShapeDisplayLevel", "ConFixedCode", "ConLineJumpCode", "ConLineJumpStyle",
    "ConLineJumpDirX", "ConLineJumpDirY", "ConLineRouteExt", "DisplayLevel",
];
// the sections whose local rows move: always, with the text, with the shape data, with the format
const ALWAYS_SECS: &[&str] = &["Hyperlink"];
const TEXT_SECS: &[&str] = &["Character", "Paragraph", "Tabs", "Field"];
const DATA_SECS: &[&str] = &["Property"];
const FORMAT_SECS: &[&str] = &["FillGradient", "LineGradient"];

// which of the old shape's single cells move
fn single_moves(name: &str, b: &Behavior) -> bool {
    if TRANSFORM.contains(&name) || ENDS.contains(&name) || ALIGN.contains(&name) || LAYOUT.contains(&name) || name == "LayerMember" {
        return true;
    }
    if name.starts_with("Lock") && name != "LockReplace" {
        return true;
    }
    match Category::of(None, name) {
        Some(Category::Line | Category::Fill) => !b.lock_format,
        Some(Category::Text) => !b.lock_text,
        None => false,
    }
}

fn section_moves(name: &str, b: &Behavior) -> bool {
    ALWAYS_SECS.contains(&name) || (TEXT_SECS.contains(&name) && !b.lock_text) || (DATA_SECS.contains(&name) && !b.lock_data) || (FORMAT_SECS.contains(&name) && !b.lock_format)
}

// write an old shape's cell into the new shape (the row made when the new shape lacks it); a
// guarded cell stays the master's
fn put(tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, c: &Cell) -> Result<(), EditError> {
    let r = match c.formula {
        Formula::Expr(f) => {
            let f = alloc::string::String::from(tx.doc().str(f));
            tx.set_formula(sheet, addr, &f).and_then(|_| tx.set_result(sheet, addr, c.value, None))
        }
        Formula::Inherited => return Ok(()),
        _ => tx.set_constant(sheet, addr, c.value, c.unit),
    };
    match r {
        Ok(()) | Err(EditError::Guarded) => Ok(()),
        Err(e) => Err(e),
    }
}

fn ensure_row(tx: &mut Transaction, sheet: SheetRef, sk: SecKey, key: RowKey, kind: Option<crate::strings::Sym>) -> Result<(), EditError> {
    if tx.doc().has_row(sheet, sk, key) {
        return Ok(());
    }
    if !tx.doc().has_section(sheet, sk) {
        let name = alloc::string::String::from(tx.doc().str(sk.name));
        // an indexed section (Character ...) of index 0, a named one
        let got = tx.add_section(sheet, &name)?;
        if got != sk {
            return Err(EditError::Invalid);
        }
    }
    let kind = kind.map(|k| alloc::string::String::from(tx.doc().str(k)));
    tx.add_row(sheet, sk, Some(key), kind.as_deref()).map(|_| ())
}

// the old shape's local cells into the new one
fn copy_cells(tx: &mut Transaction, old: ShapeRef, new: ShapeRef, b: &Behavior) -> Result<(), EditError> {
    let Some(sh) = tx.doc().sheet(SheetRef::Shape(old)).cloned() else { return Ok(()) };
    let to = SheetRef::Shape(new);
    for c in &sh.cells {
        let name = alloc::string::String::from(tx.doc().str(c.name));
        if single_moves(&name, b) {
            put(tx, to, &CellAddr::single(c.name), c)?;
        }
    }
    for sec in &sh.sections {
        if sec.del {
            continue;
        }
        let name = alloc::string::String::from(tx.doc().str(sec.name));
        if !section_moves(&name, b) {
            continue;
        }
        let sk = sec.key();
        for r in &sec.rows {
            if r.del {
                continue;
            }
            ensure_row(tx, to, sk, r.key, r.kind)?;
            for c in &r.cells {
                put(tx, to, &CellAddr::row(sk, r.key, c.name), c)?;
            }
        }
    }
    // ReplaceCopyCells: these cells whatever the locks say (resolved: the old shape shows them)
    for name in &b.copy {
        let Some(part) = crate::formula::cell_part(name, None) else { continue };
        let Some(addr) = crate::formula::bind(tx.doc(), &part) else { continue };
        let r = tx.doc().resolve(SheetRef::Shape(old), &addr);
        if !r.exists() {
            continue;
        }
        let c = Cell { name: addr.cell, value: r.value(), formula: r.formula(), unit: r.unit(), error: None, stale: false };
        if let (Some(sk), Some(rk)) = (addr.section, addr.row) {
            ensure_row(tx, to, sk, rk, None)?;
        }
        let c = if c.formula == Formula::Inherited { Cell { formula: Formula::Absent, ..c } } else { c };
        put(tx, to, &addr, &c)?;
    }
    Ok(())
}

// the old shape's own text (and its styles where it set them) into the new one
fn copy_text_and_styles(tx: &mut Transaction, old: ShapeRef, new: ShapeRef, b: &Behavior) -> Result<(), EditError> {
    let (text, styles) = {
        let doc = tx.doc();
        let sh = doc.shape(old).ok_or(EditError::NoShape)?;
        let text = if b.lock_text { None } else { sh.data().text.clone() };
        let own = sh.header().styles;
        let theirs = doc.master_shape_of(old).and_then(|m| doc.shape(m)).map(|m| m.header().styles);
        (text, (theirs != Some(own) && !b.lock_format).then_some(own))
    };
    if let Some(t) = text {
        tx.set_text(new, Some(*t))?;
    }
    if let Some(s) = styles {
        if s.line.is_some() || s.fill.is_some() || s.text.is_some() {
            tx.set_shape_styles(new, s)?;
        }
    }
    Ok(())
}

// the Relationships naming `old` name `new` instead, `new` gets old's
fn move_relations(tx: &mut Transaction, owner: Owner, old: ShapeRef, new: ShapeRef) -> Result<(), EditError> {
    let mine = rel::relations(tx.doc(), old);
    if !mine.is_empty() {
        rel::write(tx, new, &mine)?;
        rel::write(tx, old, &rel::Relations::default())?;
    }
    for s in rel::related_shapes(tx.doc(), owner) {
        if s == new || s == old {
            continue;
        }
        let mut r = rel::relations(tx.doc(), s);
        let mut changed = false;
        for g in r.groups.iter_mut() {
            for x in g.1.iter_mut() {
                if *x == old {
                    *x = new;
                    changed = true;
                }
            }
        }
        if changed {
            rel::write(tx, s, &r)?;
        }
    }
    Ok(())
}

// the place of the new shape: the old one's parent and z-order
fn take_place(tx: &mut Transaction, old: ShapeRef, new: ShapeRef) -> Result<(), EditError> {
    let (parent, pos) = {
        let doc = tx.doc();
        let parent = doc.shape(old).and_then(|s| s.parent());
        let sib: Vec<ShapeRef> = match parent {
            Some(p) => doc.shape(p).map(|x| x.children().collect()).unwrap_or_default(),
            None => doc.top_shapes(old.owner).collect(),
        };
        (parent, sib.iter().position(|&x| x == old).unwrap_or(sib.len()))
    };
    tx.move_shape(new, parent, Some(pos))
}

/// replace every shape of `shapes` that can_replace allows by an instance of `master` (a master
/// of this document: instance::import_master first), Name in language `lang` (instance::LANGS);
/// see the head of this file -> the new shapes (the others are left alone)
pub fn replace(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], master: MasterRef, lang: usize) -> Result<Vec<ShapeRef>, EditError> {
    let mut out = Vec::new();
    for &old in shapes {
        if !can_replace(tx.doc(), old, master) {
            continue;
        }
        let Owner::Page(page) = old.owner else { continue };
        // where the old one is on the page, what is glued to it
        let (pin, conns, own_ends) = {
            let doc = tx.doc();
            let f = crate::ops::frame(doc, old);
            let pin = f.centre();
            let mut conns: Vec<(ShapeRef, End, Target, [f64; 2])> = Vec::new();
            for c in glue::dependents(doc, old.owner, &[old]) {
                if c == old {
                    continue;
                }
                let g = glue::glued(doc, c);
                let Some((b, e)) = glue::ends(doc, c) else { continue };
                for (k, t) in g.iter().enumerate() {
                    let hit = match *t {
                        Target::Point { shape, .. } | Target::Shape(shape) => shape == old,
                        Target::None => false,
                    };
                    if hit {
                        conns.push((c, if k == 0 { End::Begin } else { End::End }, *t, if k == 0 { b } else { e }));
                    }
                }
            }
            let own_ends = if crate::ops::one_d(doc, old) { glue::ends(doc, old).map(|e| (glue::glued(doc, old), e)) } else { None };
            (pin, conns, own_ends)
        };
        let new = crate::instance::place(tx, page, master, pin[0], pin[1], lang)?;
        take_place(tx, old, new)?;
        let b = Behavior::of(tx.doc(), SheetRef::Shape(new));
        copy_cells(tx, old, new, &b)?;
        copy_text_and_styles(tx, old, new, &b)?;
        let mut sheets = Vec::new();
        let mut stack = alloc::vec![new];
        while let Some(s) = stack.pop() {
            sheets.push(SheetRef::Shape(s));
            if let Some(x) = tx.doc().shape(s) {
                stack.extend(x.children());
            }
        }
        eng.recalc(tx, &sheets)?;
        move_relations(tx, old.owner, old, new)?;
        // connectors glued to the old shape: to the same point of the new one, else to the whole
        let rows: Vec<u32> = glue::connection_points(tx.doc(), new).iter().map(|p| p.row).collect();
        for (c, end, t, at) in conns {
            let to = match t {
                Target::Point { row, .. } if rows.contains(&row) => Target::Point { shape: new, row },
                _ => Target::Shape(new),
            };
            if glue::glue(tx, eng, c, end, at, to).is_err() {
                glue::glue(tx, eng, c, end, at, Target::None)?;
            }
        }
        // a 1-D shape's own glue
        if let Some((targets, (b0, e0))) = own_ends {
            for (k, t) in targets.iter().enumerate() {
                if *t == Target::None {
                    continue;
                }
                let (end, at) = if k == 0 { (End::Begin, b0) } else { (End::End, e0) };
                if glue::glue(tx, eng, new, end, at, *t).is_err() {
                    glue::glue(tx, eng, new, end, at, Target::None)?;
                }
            }
        }
        glue::detach(tx, eng, old.owner, &[old])?;
        tx.delete_shapes(&[old])?;
        out.push(new);
    }
    if !out.is_empty() {
        // lists lay their items out again (a new size), containers fit
        let owner = out[0].owner;
        let all: Vec<ShapeRef> = out.iter().copied().filter(|&s| tx.doc().shape(s).is_some_and(|x| x.parent().is_none())).collect();
        structure::settle(tx, eng, owner, &all, false)?;
    }
    Ok(out)
}
