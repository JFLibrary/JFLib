// sfuncs -- the structure functions (FX-12, JD10): containers, lists, callouts, categories
//
//   CONTAINERCOUNT()                     how many containers and lists hold the shape (nested
//                                        ones too: Visio stores every enclosing one)
//   CONTAINERSHEETREF(index[, category]) the index-th of them front to back in z-order
//                                        (1-based; with a category: of those that have it)
//   CONTAINERSHEETREF(sheet, index[, category])   the same for another shape
//   CONTAINERMEMBERCOUNT()               members of the shape (a container; -1: none)
//   LISTMEMBERCOUNT()                    items of the shape (a list; 0: no list)
//   LISTORDER()                          the shape's place in its list (1-based; -1: in none)
//   LISTSHEETREF()                       the list the shape is in
//   CALLOUTCOUNT()                       how many callouts are tied to the shape
//   CALLOUTTARGETREF()                   the target of the shape (a callout)
//   HASCATEGORY(category)                whether msvShapeCategories names the category
//
// The structure lives in the Relationships cells (structure::rel); every function reads them
// through the engine, so a formula that uses one is recomputed when membership changes (the
// cell's formula changes with every add and remove).  A reference to no shape is #REF!, as in
// Visio; a shape off a page (a master's) has no structure.

use alloc::string::String;
use alloc::vec::Vec;

use super::eval::{Ev, R};
use super::parse::Func;
use super::val::{Target, Val};
use crate::cell::CellError;
use crate::document::{ShapeRef, SheetRef};
use crate::sheet::CellAddr;
use crate::structure::{self, rel};

// the shape the formula runs for
fn shape(ev: &Ev) -> Option<ShapeRef> {
    match ev.sheet {
        SheetRef::Shape(r) => Some(r),
        _ => None,
    }
}

// a shape's relations, read through the engine (the dependency is recorded)
fn relations(ev: &mut Ev, s: ShapeRef) -> rel::Relations {
    if let Some(c) = ev.doc.sym(rel::CELL) {
        let _ = super::engine::cell_value(ev.doc, ev.run, SheetRef::Shape(s), &CellAddr::single(c));
    }
    rel::relations(ev.doc, s)
}

// a shape's categories, read through the engine
fn categories(ev: &mut Ev, s: ShapeRef) -> Vec<String> {
    if let Some(a) = structure::user_addr(ev.doc, "msvShapeCategories") {
        let _ = super::engine::cell_value(ev.doc, ev.run, SheetRef::Shape(s), &a);
    }
    structure::categories(ev.doc, s)
}

fn sheet_ref(s: ShapeRef) -> Val {
    Val::Target(Target { sheet: SheetRef::Shape(s), addr: None })
}

fn count(n: usize) -> R {
    Ok(Val::Num(n as f64))
}

// the containers of a shape (member and item), front to back in z-order as Visio counts them
fn containers(ev: &mut Ev, s: ShapeRef) -> Vec<ShapeRef> {
    let r = relations(ev, s);
    let mut cs: Vec<ShapeRef> = r.get(rel::REL_ITEM).to_vec();
    for &c in r.get(rel::REL_MEMBER) {
        if !cs.contains(&c) {
            cs.push(c);
        }
    }
    let z = |c: &ShapeRef| ev.doc.tree(c.owner).and_then(|t| t.top().iter().position(|&x| x == c.slot)).unwrap_or(0);
    cs.sort_by_key(|c| core::cmp::Reverse(z(c)));
    cs
}

/// one of the structure functions with its argument nodes
pub(crate) fn call(ev: &mut Ev, f: Func, a: &[u32]) -> R {
    let me = shape(ev);
    match f {
        Func::ContainerCount => match me {
            Some(s) => count(containers(ev, s).len()),
            None => count(0),
        },
        Func::ContainerSheetRef => {
            // (index[, category]) for the shape itself, or (sheet, index[, category]) for
            // another one (an undocumented form Visio's own CFF formulas use)
            let (who, rest) = match a.first().map(|&x| ev.eval(x)) {
                Some(Ok(Val::Target(t))) if a.len() >= 2 => (
                    match t.sheet {
                        SheetRef::Shape(r) => Some(r),
                        _ => None,
                    },
                    &a[1..],
                ),
                _ => (me, a),
            };
            if rest.is_empty() || rest.len() > 2 {
                return Err(CellError::Value);
            }
            let k = ev.num(rest[0])?;
            let cat = if rest.len() == 2 { Some(ev.text(rest[1])?) } else { None };
            let Some(s) = who else { return Err(CellError::Ref) };
            if !(k >= 1.0) {
                return Err(CellError::Ref);
            }
            let mut n = 0usize;
            for c in containers(ev, s) {
                if let Some(cat) = &cat {
                    if !categories(ev, c).iter().any(|x| x.eq_ignore_ascii_case(cat.trim())) {
                        continue;
                    }
                }
                n += 1;
                if n == k as usize {
                    return Ok(sheet_ref(c));
                }
            }
            Err(CellError::Ref)
        }
        Func::ContainerMemberCount => match me {
            Some(s) if structure::is_container(ev.doc, s) => {
                let r = relations(ev, s);
                let mut n = r.get(rel::REL_LIST).to_vec();
                for &m in r.get(rel::REL_CONTAINER) {
                    if !n.contains(&m) {
                        n.push(m);
                    }
                }
                count(n.len())
            }
            _ => Ok(Val::Num(-1.0)),
        },
        Func::ListMemberCount => match me {
            Some(s) if structure::kind(ev.doc, s) == Some(structure::Kind::List) => count(relations(ev, s).get(rel::REL_LIST).len()),
            _ => count(0),
        },
        Func::ListOrder | Func::ListSheetRef => {
            let list = me.and_then(|s| relations(ev, s).get(rel::REL_ITEM).first().copied());
            match (f, me, list) {
                (Func::ListSheetRef, _, Some(l)) => Ok(sheet_ref(l)),
                (Func::ListSheetRef, ..) => Err(CellError::Ref),
                (_, Some(s), Some(l)) => {
                    let items = relations(ev, l);
                    Ok(Val::Num(items.get(rel::REL_LIST).iter().position(|&x| x == s).map_or(-1.0, |p| p as f64 + 1.0)))
                }
                _ => Ok(Val::Num(-1.0)),
            }
        }
        Func::CalloutCount => match me {
            Some(s) => count(relations(ev, s).get(rel::REL_TARGET).len()),
            None => count(0),
        },
        Func::CalloutTargetRef => {
            let Some(s) = me else { return Err(CellError::Ref) };
            match relations(ev, s).get(rel::REL_CALLOUT).first() {
                Some(&t) => Ok(sheet_ref(t)),
                None => Err(CellError::Ref),
            }
        }
        Func::HasCategory => {
            if a.len() != 1 {
                return Err(CellError::Value);
            }
            let cat = ev.text(a[0])?;
            let Some(s) = me else { return Ok(Val::bool(false)) };
            Ok(Val::bool(categories(ev, s).iter().any(|x| x.eq_ignore_ascii_case(cat.trim()))))
        }
        _ => Err(CellError::Name),
    }
}
