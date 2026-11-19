// rack -- (JD13, VL-52) equipment that snaps onto the height units of a rack: what the gestures
// of structures.rs do before membership is settled
//
//   docked(tx, eng, owner, shapes)   after a move or a drop: a shape with an outward connection
//                                    point docks on the nearest inward point in reach, anywhere
//                                    over a rack that takes it (diagram_core::dock); connectors
//                                    follow
//   resized(tx, eng, owner, shapes)  after a resize: a resized device docks again (its height
//                                    snapped to whole units by its SETATREF formula, the frame
//                                    may have moved), the devices of a resized rack too
//
// The rack and the devices are masters of stencils/rack_* (own sources): the rack has an inward
// point on every unit boundary of its left rail and is a container for the category
// RackEquipment, a device an outward point at its lower left corner.  The code knows only the
// connection points and the container: other shapes that dock (P2: doors, valves) use the same.
// The snap happens when the gesture ends; the drag itself shows no snapping yet.

use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::{dock, structure, EditError, Owner, ShapeRef, Transaction};

use crate::edit_canvas;

/// how near on paper an outward point must come to an inward one outside a rack (4 mm)
const REACH_PAPER: f64 = 4.0 / 25.4;

/// after a move or a drop of `shapes`: dock one of them, connectors follow
pub(crate) fn docked(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    let reach = dock::reach(tx.doc(), owner, REACH_PAPER);
    if dock::dock(tx, eng, owner, shapes, reach)?.is_some_and(|s| s.d != [0.0, 0.0]) {
        let along = structure::carried(tx.doc(), shapes);
        edit_canvas::follow(tx, eng, owner, &along)?;
    }
    Ok(())
}

/// after a resize of `shapes`: each resized device and each device of a resized container docks
/// again on its own
pub(crate) fn resized(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    let mut each: Vec<ShapeRef> = Vec::new();
    for &s in shapes {
        if structure::is_container(tx.doc(), s) {
            each.extend(structure::members(tx.doc(), s));
        } else {
            each.push(s);
        }
    }
    for s in each {
        docked(tx, eng, owner, &[s])?;
    }
    Ok(())
}
