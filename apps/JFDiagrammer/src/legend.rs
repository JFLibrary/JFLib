// legend -- (JD13, SH-151) the legend of a network plan counts the devices of its page
//
//   refresh(tx, eng, owner)          every legend on `owner` (a shape with User.JFD_Legend)
//                                    gets its text anew: its own name as the heading, then per
//                                    kind of device "3 × Router", sorted by name; its height
//                                    follows the lines
//
// A device is a shape with User.visLegendShape (Visio's mark for what its legend counts; the
// network masters have it through stencils/net_common/asset.inc and location.inc), its kind the
// name it was dropped with without Visio's ".ID".  structures.rs calls refresh after a drop, a
// paste and a delete, in the same undo step; moving changes no count.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::{Engine, Input};
use diagram_core::{dock, CellAddr, Document, EditError, Owner, ShapeRef, SheetRef, Text, Transaction, Unit};

// a User cell's number (None: the shape has no such cell)
fn user_num(doc: &Document, s: ShapeRef, name: &str) -> Option<f64> {
    let row = doc.sym(name)?;
    let sec = doc.sym("User")?;
    let val = doc.sym("Value")?;
    doc.resolve(SheetRef::Shape(s), &CellAddr::row(diagram_core::SecKey::one(sec), diagram_core::RowKey::Named(row), val)).num()
}

// a shape's name without Visio's ".ID" ("Router.12" -> "Router")
fn kind_of(doc: &Document, s: ShapeRef) -> String {
    let name = doc.shape(s).and_then(|x| x.header().name.or(x.header().name_u)).map(|n| String::from(doc.str(n))).unwrap_or_default();
    match name.rsplit_once('.') {
        Some((base, id)) if !base.is_empty() && !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => String::from(base),
        _ => name,
    }
}

/// every legend on `owner` lists the devices there anew
pub(crate) fn refresh(tx: &mut Transaction, eng: &mut Engine, owner: Owner) -> Result<(), EditError> {
    let (legends, counts) = {
        let doc = tx.doc();
        let shapes: Vec<ShapeRef> = doc.all_shapes(owner).collect();
        let legends: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| user_num(doc, s, "JFD_Legend").is_some_and(|v| v != 0.0)).collect();
        if legends.is_empty() {
            return Ok(());
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for &s in &shapes {
            if legends.contains(&s) || user_num(doc, s, "visLegendShape").is_none_or(|v| v == 0.0) {
                continue;
            }
            *counts.entry(kind_of(doc, s)).or_insert(0) += 1;
        }
        (legends, counts)
    };
    for l in legends {
        let mut text = kind_of(tx.doc(), l);
        for (name, n) in &counts {
            text.push_str(&format!("\n{n} × {name}"));
        }
        let now = tx.doc().shape(l).and_then(|x| x.text()).map(|t| t.chars()).unwrap_or_default();
        if now.trim_end_matches('\n') != text {
            tx.set_text(l, Some(Text::plain(&text)))?;
        }
        // the heading band is 8 mm on paper, each line about 5 mm, 3 mm below
        // (the top edge stays)
        let h = dock::reach(tx.doc(), owner, (8.0 + 5.0 * counts.len() as f64 + 3.0) / 25.4);
        let num = |tx: &mut Transaction, n: &str| {
            let a = CellAddr::single(tx.intern(n));
            (a, tx.doc().resolve(SheetRef::Shape(l), &a).num().unwrap_or(0.0))
        };
        let (ha, old) = num(tx, "Height");
        let (pa, pin) = num(tx, "PinY");
        if (old - h).abs() > 1e-9 {
            let _ = eng.user_set(tx, SheetRef::Shape(l), &ha, Input::Length(h * 25.4, Unit::Millimeters));
            let _ = eng.user_set(tx, SheetRef::Shape(l), &pa, Input::Length((pin - (h - old) * 0.5) * 25.4, Unit::Millimeters));
        }
    }
    Ok(())
}
