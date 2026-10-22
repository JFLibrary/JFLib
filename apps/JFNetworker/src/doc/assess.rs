// assess -- the undoable steps of making an assessment (A1, workspace/author.rs): the lab with
// its assessment into the project, and the start network in place of the network
//
//   set_lab(p, lab) -> Option<edit>         the project's lab (and its title, when the project
//                                           has none) as one step ("Aufgabe festlegen"), None
//                                           when it is the same
//   network(p, devices, links) -> Option<edit>   the devices and cables of the project replaced
//                                           ("Ausgangsnetz erzeugen"); their ids stay, so the
//                                           canvas, the simulator and undo agree on them
//
// The simulator follows both through Doc::apply (sim.sync: devices whose configuration changed
// start again with it).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::lab::Lab;
use jfn_model::{Device, Link, Project};
use ui::Edit;

/// the lab of the project before and after, with its title
struct SetLab {
    from: (Option<Lab>, String),
    to: (Option<Lab>, String),
}

impl Edit<Project> for SetLab {
    fn apply(&mut self, p: &mut Project) {
        p.lab = self.to.0.clone();
        p.title = self.to.1.clone();
    }
    fn revert(&mut self, p: &mut Project) {
        p.lab = self.from.0.clone();
        p.title = self.from.1.clone();
    }
    fn label(&self) -> &str {
        "Aufgabe festlegen"
    }
}

/// the project's lab as one step (a project without a title takes the lab's German one)
pub(crate) fn set_lab(p: &Project, lab: Option<Lab>) -> Option<Box<dyn Edit<Project>>> {
    if p.lab == lab {
        return None;
    }
    let title = match (&lab, p.title.trim().is_empty()) {
        (Some(l), true) => l.title.de.clone(),
        _ => p.title.clone(),
    };
    Some(Box::new(SetLab { from: (p.lab.clone(), p.title.clone()), to: (lab, title) }))
}

/// the devices and links of the project before and after
struct Network {
    from: (Vec<Device>, Vec<Link>),
    to: (Vec<Device>, Vec<Link>),
    label: String,
}

fn put(p: &mut Project, net: &(Vec<Device>, Vec<Link>)) {
    p.devices = net.0.clone();
    p.links = net.1.clone();
    p.next_id = p.next_id.max(p.max_id() + 1);
}

impl Edit<Project> for Network {
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

/// the project's devices and cables replaced (the start network) as one step, None when
/// nothing changes
pub(crate) fn network(p: &Project, devices: Vec<Device>, links: Vec<Link>) -> Option<Box<dyn Edit<Project>>> {
    if p.devices == devices && p.links == links {
        return None;
    }
    Some(Box::new(Network { from: (p.devices.clone(), p.links.clone()), to: (devices, links), label: String::from("Ausgangsnetz erzeugen") }))
}
