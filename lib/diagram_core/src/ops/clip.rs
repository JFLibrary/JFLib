// ops::clip -- copy and paste of shapes inside the app
//
//   copy(doc, shapes) -> Clip                     the shapes with their subshapes, as top-level
//                                                 shapes (a subshape takes its groups'
//                                                 transforms into its own cells)
//   paste(tx, eng, owner, &clip, offset)          the copies on a page or in a master, moved by
//                                                 `offset` -> the new top-level shapes
//
// Pasted shapes get new IDs; the formulas of the pasted shapes that name a copied shape by
// Sheet.N (or by a NameU that ends in .N) name its copy, Names and NameUs that end in .<old ID>
// end in the new one, UniqueIDs are dropped, the Connect entries between copied shapes come
// along with the new IDs.  Glue to a shape outside the copy is let go: the glued cells keep
// their values as constants, the Connect entry stays behind.  A Clip holds the document's
// string handles and refers to its masters and styles by ID: it pastes into the document it
// was copied from (an app with several documents copies across them through a file format).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{bounds, ends, glue_cells, norm_angle, one_d, outermost, parent_xf, text, val};
use crate::cell::{Cell, Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::names;
use crate::page::Connect;
use crate::shape::ShapeData;
use crate::sheet::{CellAddr, Sheet};
use crate::strings::Sym;

#[derive(Clone, Debug)]
struct Item {
    // the shape's ID where it was copied
    id: u32,
    // the item of its group (None: a top-level shape of the copy)
    parent: Option<usize>,
    data: ShapeData,
}

/// copied shapes (copy), ready to paste
#[derive(Clone, Debug, Default)]
pub struct Clip {
    // groups before their subshapes, top-level shapes in z-order
    items: Vec<Item>,
    // the Connect entries from copied shapes (IDs where they were copied)
    connects: Vec<Connect>,
    bounds: Option<[f64; 4]>,
}

impl Clip {
    /// whether nothing was copied
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// the number of top-level shapes in it
    pub fn len(&self) -> usize {
        self.items.iter().filter(|i| i.parent.is_none()).count()
    }

    /// the box of the copied shapes on the page where they were copied
    pub fn bounds(&self) -> Option<[f64; 4]> {
        self.bounds
    }
}

// a single cell of a sheet set to a constant
fn put_const(sheet: &mut Sheet, name: Sym, v: f64) {
    match sheet.cells.iter_mut().find(|c| c.name == name) {
        Some(c) => {
            c.value = Value::Num(v);
            c.formula = Formula::Absent;
            c.error = None;
            c.stale = false;
        }
        None => sheet.cells.push(Cell::constant(name, Value::Num(v), None)),
    }
}

/// copy shapes with their subshapes (shapes inside another one of the list go with it); a
/// subshape becomes a top-level shape with its groups' transforms in its own pin, angle and
/// flips (1D: its ends)
pub fn copy(doc: &Document, shapes: &[ShapeRef]) -> Clip {
    let list = outermost(doc, shapes);
    let mut clip = Clip { bounds: bounds(doc, &list), ..Clip::default() };
    let mut ids: BTreeSet<(Owner, u32)> = BTreeSet::new();
    for &root in &list {
        let first = clip.items.len();
        let mut stack: Vec<(ShapeRef, Option<usize>)> = alloc::vec![(root, None)];
        while let Some((s, parent)) = stack.pop() {
            let Some(sh) = doc.shape(s) else { continue };
            let i = clip.items.len();
            clip.items.push(Item { id: sh.id(), parent, data: sh.data().clone() });
            ids.insert((s.owner, sh.id()));
            let kids: Vec<ShapeRef> = sh.children().collect();
            stack.extend(kids.into_iter().rev().map(|k| (k, Some(i))));
        }
        // a subshape: into page coordinates
        if doc.shape(root).and_then(|s| s.parent()).is_some() {
            let m = parent_xf(doc, root);
            let sheet = SheetRef::Shape(root);
            let data = &mut clip.items[first].data.sheet;
            let mirrored = m.det() < 0.0;
            if one_d(doc, root) {
                let (b, e) = ends(doc, root);
                let (b, e) = (m.apply(b), m.apply(e));
                put_const(data, names::BEGIN_X, b[0]);
                put_const(data, names::BEGIN_Y, b[1]);
                put_const(data, names::END_X, e[0]);
                put_const(data, names::END_Y, e[1]);
                if mirrored {
                    let fy = val(doc, sheet, names::FLIP_Y) != 0.0;
                    put_const(data, names::FLIP_Y, if fy { 0.0 } else { 1.0 });
                }
            } else {
                // the parent's turn and mirror: R(phi), times diag(1, -1) when mirrored
                let phi = crate::formula::math::atan2(m.b, m.a);
                let p = m.apply([val(doc, sheet, names::PIN_X), val(doc, sheet, names::PIN_Y)]);
                let a = val(doc, sheet, names::ANGLE);
                put_const(data, names::PIN_X, p[0]);
                put_const(data, names::PIN_Y, p[1]);
                put_const(data, names::WIDTH, val(doc, sheet, names::WIDTH));
                put_const(data, names::HEIGHT, val(doc, sheet, names::HEIGHT));
                put_const(data, names::ANGLE, norm_angle(if mirrored { phi - a } else { phi + a }));
                put_const(data, names::FLIP_X, val(doc, sheet, names::FLIP_X));
                let fy = val(doc, sheet, names::FLIP_Y) != 0.0;
                put_const(data, names::FLIP_Y, if fy != mirrored { 1.0 } else { 0.0 });
            }
        }
    }
    let owners: BTreeSet<Owner> = list.iter().map(|s| s.owner).collect();
    for owner in owners {
        for c in super::connects(doc, owner) {
            if ids.contains(&(owner, c.from_sheet)) {
                clip.connects.push(c.clone());
            }
        }
    }
    clip
}

// a name that ends in .<old ID> with the new ID
fn renamed(name: &str, old: u32, new: u32) -> Option<String> {
    let suffix = format!(".{old}");
    name.strip_suffix(&suffix).map(|stem| format!("{stem}.{new}"))
}

/// paste copied shapes on a page or in a master (into the document they were copied from),
/// on top of its shapes and moved by `offset` (page) -> the new top-level shapes
pub fn paste(tx: &mut Transaction, eng: &mut Engine, owner: Owner, clip: &Clip, offset: [f64; 2]) -> Result<Vec<ShapeRef>, EditError> {
    tx.doc().tree(owner).ok_or(EditError::NoPage)?;
    for it in &clip.items {
        if let Some(m) = it.data.header.master {
            tx.doc().master_by_id(m).ok_or(EditError::NoMaster)?;
        }
    }
    // the shapes
    let mut made: Vec<ShapeRef> = Vec::with_capacity(clip.items.len());
    let mut ids: BTreeMap<u32, u32> = BTreeMap::new();
    let mut names_u: BTreeMap<String, String> = BTreeMap::new();
    for it in &clip.items {
        let mut data = it.data.clone();
        data.header.unique_id = None;
        let parent = it.parent.map(|i| made[i]);
        let s = tx.add_shape_with(owner, parent, data)?;
        let new = tx.doc().shape(s).ok_or(EditError::NoShape)?.id();
        ids.insert(it.id, new);
        made.push(s);
        // Name and NameU that end in the old ID
        let h = tx.doc().shape(s).ok_or(EditError::NoShape)?.header().clone();
        let nu = h.name_u.map(|n| String::from(tx.doc().str(n)));
        let nn = h.name.map(|n| String::from(tx.doc().str(n)));
        let new_nu = nu.as_deref().and_then(|n| renamed(n, it.id, new));
        let new_n = nn.as_deref().and_then(|n| renamed(n, it.id, new));
        if new_nu.is_some() || new_n.is_some() {
            if let (Some(old), Some(n)) = (&nu, &new_nu) {
                names_u.insert(old.clone(), n.clone());
            }
            let mut h2 = h.clone();
            if let Some(n) = new_nu {
                h2.name_u = Some(tx.intern(&n));
            }
            if let Some(n) = new_n {
                h2.name = Some(tx.intern(&n));
            }
            tx.set_shape_header(s, h2)?;
        }
    }
    // their formulas: the copies' IDs and names
    for &s in &made {
        let sheet = SheetRef::Shape(s);
        let mut edits: Vec<(CellAddr, String)> = Vec::new();
        {
            let doc = tx.doc();
            let Some(sh) = doc.sheet(sheet) else { continue };
            let mut visit = |addr: CellAddr, c: &Cell| {
                let Formula::Expr(f) = c.formula else { return };
                let f = doc.str(f);
                let list = text::items(f);
                let mut ed: Vec<((usize, usize), String)> = Vec::new();
                for item in &list {
                    let text::Item::Sheet { text: t, range, foreign: false, quoted } = item else { continue };
                    if *quoted {
                        let inner = &t[1..t.len().saturating_sub(1)];
                        if let Some(n) = names_u.get(inner) {
                            ed.push((*range, format!("'{n}'")));
                        }
                    } else if let Some(n) = t.strip_prefix("Sheet.").and_then(|n| n.parse::<u32>().ok()) {
                        if let Some(m) = ids.get(&n) {
                            ed.push((*range, format!("Sheet.{m}")));
                        }
                    } else if let Some(n) = names_u.get(*t) {
                        ed.push((*range, n.clone()));
                    }
                }
                if !ed.is_empty() {
                    edits.push((addr, text::replace(f, ed)));
                }
            };
            for c in &sh.cells {
                visit(CellAddr::single(c.name), c);
            }
            for sec in &sh.sections {
                if sec.del {
                    continue;
                }
                let sk = sec.key();
                for c in &sec.cells {
                    visit(CellAddr::section(sk, c.name), c);
                }
                for r in &sec.rows {
                    if r.del {
                        continue;
                    }
                    for c in &r.cells {
                        visit(CellAddr::row(sk, r.key, c.name), c);
                    }
                }
            }
        }
        for (a, f) in edits {
            tx.force_formula(sheet, &a, &f)?;
        }
    }
    // glue: inside the copy it comes along, to the outside it is let go
    for c in &clip.connects {
        let Some(&from) = ids.get(&c.from_sheet) else { continue };
        match ids.get(&c.to_sheet) {
            Some(&to) => tx.add_connect(owner, Connect { from_sheet: from, to_sheet: to, ..c.clone() })?,
            None => {
                let Some(cell) = c.from_cell else { continue };
                let Some(s) = tx.doc().shape_by_id(owner, from) else { continue };
                let sheet = SheetRef::Shape(s);
                for a in glue_cells(tx.doc(), cell) {
                    let r = tx.doc().resolve(sheet, &a);
                    if r.exists() {
                        let (v, u) = (r.value(), r.unit());
                        tx.force_constant(sheet, &a, v, u)?;
                    }
                }
            }
        }
    }
    // the top-level copies moved by the offset (formulas keep their formula: their value moves,
    // the recalculation then computes the ones it can)
    let roots: Vec<ShapeRef> = clip.items.iter().zip(&made).filter(|(i, _)| i.parent.is_none()).map(|(_, s)| *s).collect();
    if offset != [0.0, 0.0] {
        for &s in &roots {
            let sheet = SheetRef::Shape(s);
            let cells = if one_d(tx.doc(), s) {
                [(names::BEGIN_X, 0), (names::BEGIN_Y, 1), (names::END_X, 0), (names::END_Y, 1), (names::PIN_X, 0), (names::PIN_Y, 1)].to_vec()
            } else {
                [(names::PIN_X, 0), (names::PIN_Y, 1)].to_vec()
            };
            for (cell, axis) in cells {
                let a = CellAddr::single(cell);
                let r = tx.doc().resolve(sheet, &a);
                let Some(v) = r.num() else { continue };
                let v = Value::Num(v + offset[axis]);
                match r.formula() {
                    Formula::Expr(_) => tx.set_result(sheet, &a, v, None)?,
                    _ => {
                        let u = r.unit();
                        tx.force_constant(sheet, &a, v, u)?;
                    }
                }
            }
        }
    }
    let sheets: Vec<SheetRef> = made.iter().map(|&s| SheetRef::Shape(s)).collect();
    eng.recalc(tx, &sheets)?;
    Ok(roots)
}
