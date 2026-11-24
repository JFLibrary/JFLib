// structure::rel -- the Relationships cell: which shapes a shape is structurally tied to
//
//   relations(doc, shape) -> Relations      what the cell names, by relationship type
//   write(tx, shape, &relations)            the cell written the way Visio writes it
//
// Visio keeps container membership, list order and callout targets in the single cell
// Relationships of both shapes (Z2, Z3; learn.microsoft.com "Relationships Cell (Shape Layout
// Section)"; a plain cell under <Shape>, V="0"):
//
//   container:  SUM(DEPENDSON(1,Sheet.12!SheetRef(),Sheet.15!SheetRef()))   its members
//   member:     SUM(DEPENDSON(4,Sheet.7!SheetRef()))                         its containers
//
//   1 members of this container        4 containers of this shape
//   2 items of this list (in order)    5 the list of this item
//   3 callouts on this shape           6 the target of this callout
//   7 .. 10 the container on whose left / right / top / bottom edge the shape sits (JD21:
//           written and read by structure::edge; the container names it as a member, type 1)
//   11 the list this list overlaps (a cross-functional flowchart's lanes and phases)
//
// One DEPENDSON per type, the type first, the shapes after it; Visio leaves an empty term
// (SUM(DEPENDSON(4))) after the last member left, which means nothing.  The order inside a term
// is kept: a list's items are written in list order (how Visio stores list order is not
// documented; the items also stand in that order on the page, so both readings agree).  The cell has no other meaning: the formula engine evaluates it to 0 and records
// the dependencies, which is what makes Visio recompute the shapes that read the structure
// (CONTAINERSHEETREF, LISTORDER ...).  Both sides are written; a side that is missing or
// names a shape that is gone is repaired by the editing functions (structure::repair).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::{Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::sheet::CellAddr;

/// the relationship types of DEPENDSON's first argument: a container names its members
pub const REL_CONTAINER: u32 = 1;
/// a list names its items (in list order)
pub const REL_LIST: u32 = 2;
/// a target names its callouts
pub const REL_TARGET: u32 = 3;
/// a member names its containers
pub const REL_MEMBER: u32 = 4;
/// an item names its list
pub const REL_ITEM: u32 = 5;
/// a callout names its target
pub const REL_CALLOUT: u32 = 6;

/// the type the other side writes for a type (None: kept as it is, 7 .. 11)
pub fn back(kind: u32) -> Option<u32> {
    match kind {
        REL_CONTAINER => Some(REL_MEMBER),
        REL_MEMBER => Some(REL_CONTAINER),
        REL_LIST => Some(REL_ITEM),
        REL_ITEM => Some(REL_LIST),
        REL_TARGET => Some(REL_CALLOUT),
        REL_CALLOUT => Some(REL_TARGET),
        _ => None,
    }
}

/// what a Relationships cell names: per type the shapes in the order written
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Relations {
    pub groups: Vec<(u32, Vec<ShapeRef>)>,
}

impl Relations {
    /// the shapes of type `kind`
    pub fn get(&self, kind: u32) -> &[ShapeRef] {
        self.groups.iter().find(|g| g.0 == kind).map_or(&[], |g| &g.1[..])
    }

    /// the list of type `kind`, made when missing
    pub fn get_mut(&mut self, kind: u32) -> &mut Vec<ShapeRef> {
        if let Some(i) = self.groups.iter().position(|g| g.0 == kind) {
            return &mut self.groups[i].1;
        }
        self.groups.push((kind, Vec::new()));
        let n = self.groups.len() - 1;
        &mut self.groups[n].1
    }

    /// whether it names nothing
    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.1.is_empty())
    }
}

/// the cell's name
pub(crate) const CELL: &str = "Relationships";

fn addr(doc: &Document) -> Option<CellAddr> {
    doc.sym(CELL).map(CellAddr::single)
}

/// the formula text of a shape's Relationships cell (inheritance included; masters carry none
/// in practice, but a copy may)
pub fn formula(doc: &Document, shape: ShapeRef) -> Option<&str> {
    let a = addr(doc)?;
    match doc.resolve(SheetRef::Shape(shape), &a).formula() {
        Formula::Expr(f) => Some(doc.str(f)),
        _ => None,
    }
}

/// what the Relationships cell of `shape` names (shapes that are gone are left out)
pub fn relations(doc: &Document, shape: ShapeRef) -> Relations {
    let Some(f) = formula(doc, shape) else { return Relations::default() };
    let owner = shape.owner;
    let mut out = Relations::default();
    for (kind, ids) in parse(f) {
        let list = out.get_mut(kind);
        for id in ids {
            if let Some(r) = doc.shape_by_id(owner, id) {
                if doc.shape(r).is_some_and(|s| !s.is_deleted()) && !list.contains(&r) {
                    list.push(r);
                }
            }
        }
    }
    out.groups.retain(|g| !g.1.is_empty());
    out
}

/// the DEPENDSON groups of a formula: (type, shape IDs named by Sheet.N!)
pub fn parse(f: &str) -> Vec<(u32, Vec<u32>)> {
    let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
    let up = f.to_ascii_uppercase();
    let b = up.as_bytes();
    let mut i = 0;
    while let Some(k) = up[i..].find("DEPENDSON(") {
        let start = i + k + "DEPENDSON(".len();
        // the call's extent: up to the matching parenthesis
        let mut depth = 1;
        let mut j = start;
        while j < b.len() && depth > 0 {
            match b[j] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'"' => {
                    j += 1;
                    while j < b.len() && b[j] != b'"' {
                        j += 1;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let args = &f[start..j.saturating_sub(1).max(start)];
        let mut parts = args.splitn(2, ',');
        let kind = parts.next().and_then(|t| t.trim().parse::<f64>().ok()).filter(|x| *x >= 0.0).map(|x| x as u32);
        if let Some(kind) = kind {
            let mut ids = Vec::new();
            for p in parts.next().unwrap_or("").split(',') {
                if let Some(id) = sheet_id(p.trim()) {
                    ids.push(id);
                }
            }
            match out.iter_mut().find(|g| g.0 == kind) {
                Some(g) => g.1.extend(ids),
                None => out.push((kind, ids)),
            }
        }
        i = j.max(start);
    }
    out
}

// "Sheet.12!SheetRef()" -> 12
fn sheet_id(p: &str) -> Option<u32> {
    let rest = p.strip_prefix("Sheet.").or_else(|| p.strip_prefix("sheet.")).or_else(|| p.strip_prefix("SHEET."))?;
    let end = rest.find('!')?;
    rest[..end].parse().ok()
}

/// the formula Visio writes for these relations (None: nothing to name)
pub fn formula_of(doc: &Document, rel: &Relations) -> Option<String> {
    let mut groups: Vec<String> = Vec::new();
    // Visio's order: by type
    let mut kinds: Vec<u32> = rel.groups.iter().filter(|g| !g.1.is_empty()).map(|g| g.0).collect();
    kinds.sort_unstable();
    kinds.dedup();
    for k in kinds {
        let mut s = format!("DEPENDSON({k}");
        for &r in rel.get(k) {
            let Some(sh) = doc.shape(r) else { continue };
            s.push_str(&format!(",Sheet.{}!SheetRef()", sh.id()));
        }
        s.push(')');
        groups.push(s);
    }
    if groups.is_empty() {
        return None;
    }
    Some(format!("SUM({})", groups.join(",")))
}

/// write a shape's Relationships cell (over a GUARD, as Visio does: the cell is the
/// structure's, not the user's)
pub fn write(tx: &mut Transaction, shape: ShapeRef, rel: &Relations) -> Result<(), EditError> {
    let a = tx.single(CELL);
    let sheet = SheetRef::Shape(shape);
    let want = formula_of(tx.doc(), rel);
    let now = formula(tx.doc(), shape).map(String::from);
    if want == now {
        return Ok(());
    }
    match want {
        Some(f) => {
            tx.force_formula(sheet, &a, &f)?;
            // the value is 0 whatever it names; written now so the cell is never stale in a
            // file saved before the engine ran
            tx.set_result(sheet, &a, Value::Num(0.0), None)
        }
        None => {
            // nothing left: the master's (none in practice), else a plain 0 as Visio leaves it
            let inherited = {
                let doc = tx.doc();
                doc.master_shape_of(shape).is_some_and(|m| doc.resolve(SheetRef::Shape(m), &a).exists())
            };
            if inherited {
                tx.force_constant(sheet, &a, Value::Num(0.0), None)
            } else {
                tx.inherit_cell(sheet, &a)
            }
        }
    }
}

/// every shape of a page or master whose Relationships cell names something
pub fn related_shapes(doc: &Document, owner: Owner) -> Vec<ShapeRef> {
    doc.all_shapes(owner).filter(|&s| formula(doc, s).is_some_and(|f| f.to_ascii_uppercase().contains("DEPENDSON("))).collect()
}
