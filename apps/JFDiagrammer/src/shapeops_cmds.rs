// shapeops_cmds -- (JD21) the shape operations (SP-08) and Ausrichtungsfeld aktualisieren (SP-16):
// Start › Anordnen › Gruppieren › Vorgänge: Gesamtmenge, Kombinieren, In Einzelmengen zerlegen,
// Schnittmenge bilden, Subtrahieren, Ausrichtungsfeld aktualisieren.  Visio has them under
// Entwicklertools › Shape-Design › Vorgänge; that register comes with JD23, until then the submenu
// of Gruppieren holds them (ribbon.rs GROUP_MENU_ITEMS).
//
//   available(app, id)     whether the command can run on the selection (diagram_core::shapeops::
//                          available: Gesamtmenge and In Einzelmengen zerlegen one shape with an
//                          area, Kombinieren two with geometry, Schnittmenge bilden two with an
//                          area, Subtrahieren the primary shape with an area and another one;
//                          Ausrichtungsfeld aktualisieren any selection)
//   run(app, ui, id)       the operation as one undo step named after the command; the result
//                          is selected afterwards (nothing when the result is empty)
//
// The primary shape (the first one selected, app.se.primary) is the first input: its format and
// text go to the result, Subtrahieren takes the others away from it (diagram_core shapeops.rs
// has Visio's rules and what is assumed).  Connectors glued to the inputs let go of them;
// connectors glued to a group whose box changed follow (edit_canvas::follow).

use alloc::vec::Vec;

use diagram_core::shapeops::{self, Operation};
use diagram_core::{Owner, ShapeRef};
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, key_of};
use crate::edit_doc;
use crate::Diagrammer;

// a selection larger than this is checked by its size only
const CHECK_MAX: usize = 64;

// states() asks every pass for each command: the answers for the five operations are kept for one
// state of the drawing and the selection (key: drawing serial, model version, edit revision, primary, selection)
static AVAIL: rt::sync::SpinLock<(u64, u8)> = rt::sync::SpinLock::new((0, 0));

const OPS: [u32; 5] = [OP_UNION, OP_COMBINE, OP_FRAGMENT, OP_INTERSECT, OP_SUBTRACT];

fn state_key(app: &Diagrammer) -> u64 {
    // FNV-1a over what the answers depend on
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x0100_0000_01b3);
    };
    eat(app.opened.as_ref().map_or(0, |o| o.serial as u64));
    eat(app.model.ver as u64);
    eat(app.opened.as_ref().map_or(0, |o| o.ed.rev as u64));
    eat(app.se.primary);
    for &k in &app.se.sel {
        eat(k);
    }
    h | 1
}

fn operation(id: u32) -> Option<Operation> {
    Some(match id {
        OP_UNION => Operation::Union,
        OP_COMBINE => Operation::Combine,
        OP_FRAGMENT => Operation::Fragment,
        OP_INTERSECT => Operation::Intersect,
        OP_SUBTRACT => Operation::Subtract,
        _ => return None,
    })
}

// the selection with the primary shape first
fn ordered(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(page) = app.model.page else { return Vec::new() };
    let mut keys: Vec<u64> = app.se.sel.iter().copied().collect();
    if let Some(i) = keys.iter().position(|&k| k == app.se.primary) {
        let k = keys.remove(i);
        keys.insert(0, k);
    }
    keys.into_iter().map(|k| edit_canvas::shape_of(page, k)).collect()
}

/// whether command `id` can run now (true for commands of other modules)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    if id == UPDATE_ALIGN_BOX {
        return app.opened.is_some() && !app.se.sel.is_empty();
    }
    let Some(bit) = OPS.iter().position(|&x| x == id) else { return true };
    let Some(o) = app.opened.as_ref() else { return false };
    let n = app.se.sel.len();
    if n == 0 {
        return false;
    }
    if n > CHECK_MAX {
        return true;
    }
    let key = state_key(app);
    let mut cache = AVAIL.lock();
    if cache.0 != key {
        let shapes = ordered(app);
        let mut mask = 0u8;
        for (i, &c) in OPS.iter().enumerate() {
            if operation(c).is_some_and(|op| shapeops::available(&o.doc, &shapes, op)) {
                mask |= 1 << i;
            }
        }
        *cache = (key, mask);
    }
    cache.1 & (1 << bit) != 0
}

/// a command of this module
pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let shapes = ordered(app);
    let owner = app.model.page.map(Owner::Page);
    let (Some(o), Some(ow), false) = (app.opened.as_mut(), owner, shapes.is_empty()) else { return };
    let label = crate::commands::label(id);
    let made = match operation(id) {
        Some(op) => edit_doc::edit(o, label, true, |tx, eng| shapeops::operate(tx, eng, &shapes, op)),
        None => edit_doc::edit(o, label, true, |tx, eng| {
            for &s in &shapes {
                shapeops::update_alignment_box(tx, eng, s)?;
            }
            edit_canvas::follow(tx, eng, ow, &shapes)?;
            Ok(shapes.clone())
        }),
    };
    // (refused: available() said no first, the selection stays)
    if let Ok(made) = made {
        let keys: Vec<u64> = made.iter().map(|&s| key_of(s)).collect();
        app.se.select(&keys);
    }
    ui.repass();
}
