// wlan -- the WLAN settings of the project as undo steps: a device's networks or the network
// it joins (Device::wlan), the scale of the canvas (Project::dip_per_m)
//
//   set(doc, dev, label, f) -> bool     a change of a device's WLAN settings (f gets a copy to
//                                       change; an empty result is None in the file), applied
//   scale(doc, dip_per_m) -> bool       the scale (0: the default of 10), applied
//
// The simulator follows each step without restarting the device (doc/sim.rs: Action::Wlan,
// Action::Scale): a station looks for its network again, an AP's stations whose network
// changed leave it.

use alloc::boxed::Box;

use jfn_model::{Project, Wlan};
use ui::Edit;

use super::{edits, Doc};

/// change device `dev`'s WLAN settings with `f` (one undo step; false: nothing changed)
pub(crate) fn set(doc: &mut Doc, dev: u32, label: &str, f: impl FnOnce(&mut Wlan)) -> bool {
    let Some(d) = doc.proj.device(dev) else { return false };
    let mut w = d.wlan.clone().unwrap_or_default();
    f(&mut w);
    let new = if w.is_empty() { None } else { Some(w) };
    if new == d.wlan {
        return false;
    }
    let Some(e) = edits::device_edit(&doc.proj, dev, label, |x| x.wlan = new) else { return false };
    doc.apply(e);
    true
}

struct Scale {
    from: u32,
    to: u32,
}

impl Edit<Project> for Scale {
    fn apply(&mut self, p: &mut Project) {
        p.dip_per_m = self.to;
    }
    fn revert(&mut self, p: &mut Project) {
        p.dip_per_m = self.from;
    }
    fn label(&self) -> &str {
        "Maßstab ändern"
    }
}

/// the scale of the canvas in world units per metre (one undo step; false: unchanged)
pub(crate) fn scale(doc: &mut Doc, dip_per_m: u32) -> bool {
    if jfn_model::wlan::dip_per_m(&doc.proj) == dip_per_m {
        return false;
    }
    let from = doc.proj.dip_per_m;
    doc.apply(Box::new(Scale { from, to: dip_per_m }));
    true
}
