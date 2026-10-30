// phys -- the changes of the physical view as undo steps (P1): places added, renamed, moved,
// resized and removed, devices to another room, rack unit or desk, a cable's length by hand,
// the optic in a port's cage
//
//   step(doc, label, f) -> Result<bool, PhysErr>   runs f on the project (one of
//                                    jfn_model::phys's edits); what it changed -- the places, the
//                                    devices' spots and optics, the cables' lengths and kinds --
//                                    becomes one undo step (false: nothing changed, no step)
//   live(doc, f) / since(doc, before, label)        a drag: f changes the project at once
//                                    (no step, no simulator), the step from `before` comes when it
//                                    ends; before = snapshot(doc)
//   names()                          the default tree's names in the window's language (the first
//                                    edit of a project without places stores the tree with them)
//
// A step keeps both sides as snapshots of what the physical view can change, so undo and redo
// put back exactly what was; the simulator follows each (doc/sim.rs: the automatic lengths of
// the cables, the delays that follow from them, the optics).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::phys::{PhysErr, Place, Spot};
use jfn_model::{CableKind, Optic, Project};
use ui::Edit;

use super::Doc;
use crate::app::tr::t;

/// what the physical view can change
#[derive(Clone, PartialEq)]
pub(crate) struct Snap {
    places: Vec<Place>,
    devs: Vec<(u32, Spot, BTreeMap<String, Optic>)>,
    links: Vec<(u32, u32, CableKind)>,
}

/// the physical side of the project now
pub(crate) fn snapshot(p: &Project) -> Snap {
    Snap {
        places: p.places.clone(),
        devs: p.devices.iter().map(|d| (d.id, d.phys.clone(), d.optics.clone())).collect(),
        links: p.links.iter().map(|l| (l.id, l.len_m, l.cable)).collect(),
    }
}

fn put(p: &mut Project, s: &Snap) {
    p.places = s.places.clone();
    for (id, spot, optics) in &s.devs {
        if let Some(d) = p.device_mut(*id) {
            d.phys = spot.clone();
            d.optics = optics.clone();
        }
    }
    for (id, len, cable) in &s.links {
        if let Some(l) = p.link_mut(*id) {
            l.len_m = *len;
            l.cable = *cable;
        }
    }
}

struct Step {
    from: Snap,
    to: Snap,
    label: String,
}

impl Edit<Project> for Step {
    fn apply(&mut self, p: &mut Project) {
        put(p, &self.to);
    }
    fn revert(&mut self, p: &mut Project) {
        put(p, &self.from);
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// the names of the default tree in the window's language
pub(crate) fn names() -> [&'static str; 4] {
    [t("Standort"), t("Gebäude"), t("Erdgeschoss"), t("Raum")]
}

/// one edit of the physical view as an undo step (applied); Ok(false): nothing changed
pub(crate) fn step(doc: &mut Doc, label: &str, f: impl FnOnce(&mut Project) -> Result<(), PhysErr>) -> Result<bool, PhysErr> {
    let before = snapshot(&doc.proj);
    let mut p = doc.proj.clone();
    f(&mut p)?;
    let after = snapshot(&p);
    if after == before {
        return Ok(false);
    }
    put(&mut doc.proj, &after);
    doc.record(Box::new(Step { from: before, to: after, label: String::from(label) }));
    Ok(true)
}

/// a drag in progress changes the project at once (no undo step, the simulator waits)
pub(crate) fn live(doc: &mut Doc, f: impl FnOnce(&mut Project)) {
    f(&mut doc.proj);
    doc.touch();
}

/// the step of a drag that changed the project live since `before` (None: it is the same)
pub(crate) fn since(doc: &mut Doc, before: Snap, label: &str) -> bool {
    let after = snapshot(&doc.proj);
    if after == before {
        return false;
    }
    doc.record(Box::new(Step { from: before, to: after, label: String::from(label) }));
    true
}

/// back to a snapshot without a step (a drag given up or refused)
pub(crate) fn restore(doc: &mut Doc, s: &Snap) {
    put(&mut doc.proj, s);
    doc.touch();
}
