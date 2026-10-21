// doc -- the open project: the jfn_model::Project, its undo history, the file it came from and
// the simulator that runs it
//
//   Doc::new()                      an empty project ("Unbenannt")
//   apply(edit) / record(edit)      a change through ui::History (record: already applied)
//   apply_adopted(edit, dev)        a configuration change the simulator runs already (undo
//                                   and redo restart the device with the other one)
//   apply_kept(edit, dev)           a change the running device has already and keeps through
//                                   undo and redo: a saved configuration, a service switched
//   undo() / redo()                 rev counts every change of the project (the canvas
//                                   rebuilds its scene when it differs)
//   replace(project, path)          a loaded or new project: a fresh history and engine, saved
//   dirty() / saved(path)           unsaved changes; the file was written
//   file_name()                     "Büro.jfnet", "Unbenannt" for a project without a file (a
//                                   lab of the app: its file's name, `origin`)
//
// After every change through the history (edits, undo, redo) the simulator follows the project
// (sim.sync): the difference becomes jfn_sim Actions, so undo and redo send the opposite ones.
// Live changes (touch: a drag in progress, the view) change nothing the engine knows.
//
// edits.rs has the undoable changes (devices, notes, links, cables, waypoints, power,
// configuration), file.rs reading and writing .jfnet files, the file dialogs and the "save
// changes?" question, sim.rs the engine behind the editor, config.rs the settings that change
// a device's configuration (the inspector's quick fields), console.rs the consoles of the
// devices and what the engine reports to them, autosave.rs the copies of unsaved work and
// their recovery.

pub(crate) mod autosave;
pub(crate) mod config;
pub(crate) mod console;
pub(crate) mod edits;
pub(crate) mod file;
pub(crate) mod sim;

use alloc::boxed::Box;
use alloc::string::String;

use jfn_model::Project;
use ui::{Edit, History};

use sim::Sim;

/// undo steps kept per project
const UNDO_LIMIT: usize = 200;

/// an edit of device `dev` the running device has already (Doc::apply_kept): undo and redo
/// make the simulator follow it without restarting the device
struct Kept {
    dev: u32,
    e: Box<dyn Edit<Project>>,
}

impl Kept {
    /// the device of a kept step (History::undo_value / redo_value)
    fn dev_of(v: Option<&dyn core::any::Any>) -> Option<u32> {
        v.and_then(|v| v.downcast_ref::<Kept>()).map(|k| k.dev)
    }
}

impl Edit<Project> for Kept {
    fn apply(&mut self, p: &mut Project) {
        self.e.apply(p);
    }
    fn revert(&mut self, p: &mut Project) {
        self.e.revert(p);
    }
    fn label(&self) -> &str {
        self.e.label()
    }
    fn value(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

pub(crate) struct Doc {
    pub proj: Project,
    pub hist: History<Project>,
    /// the file (None: never saved)
    pub path: Option<String>,
    /// a lab of the app opened without a file: the name of its file (title, save dialog)
    pub origin: Option<String>,
    /// counts every change of proj (edits, undo, redo, replace)
    pub rev: u32,
    /// the simulator running proj
    pub sim: Sim,
}

impl Doc {
    pub(crate) fn new() -> Doc {
        let proj = Project::new("", 1);
        let sim = Sim::new(&proj);
        Doc { proj, hist: History::new(UNDO_LIMIT), path: None, origin: None, rev: 1, sim }
    }

    /// a project from a file (or a new one): a fresh history and engine, saved; the engine
    /// starts with Net::after_load
    pub(crate) fn replace(&mut self, proj: Project, path: Option<String>) {
        self.sim = Sim::new(&proj);
        self.proj = proj;
        self.hist = History::new(UNDO_LIMIT);
        self.path = path;
        self.origin = None;
        self.touch();
    }

    /// the project changed outside the history (a live move, the view): the canvas redraws
    pub(crate) fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    // a change the engine must follow
    fn changed(&mut self) {
        self.touch();
        self.sim.sync(&self.proj);
    }

    /// run an edit and keep it as the newest undo step
    pub(crate) fn apply(&mut self, e: Box<dyn Edit<Project>>) {
        self.hist.apply(&mut self.proj, e);
        self.changed();
    }

    /// keep an edit that was already applied to proj
    pub(crate) fn record(&mut self, e: Box<dyn Edit<Project>>) {
        self.hist.record(e);
        self.changed();
    }

    /// run an edit of device `dev`'s configuration that the simulator runs already (applied
    /// live, config.rs): the engine does not restart the device for it (undo and redo do)
    pub(crate) fn apply_adopted(&mut self, e: Box<dyn Edit<Project>>, dev: u32) {
        self.hist.apply(&mut self.proj, e);
        if let Some(d) = self.proj.device(dev) {
            self.sim.adopt(d);
        }
        self.changed();
    }

    /// run an edit of device `dev` that the running device has already ("copy running-config
    /// startup-config", a service switched on or off): the simulator follows it without a
    /// restart, also when it is undone or redone (Sim::keep)
    pub(crate) fn apply_kept(&mut self, e: Box<dyn Edit<Project>>, dev: u32) {
        self.hist.apply(&mut self.proj, Box::new(Kept { dev, e }));
        self.keep(dev);
        self.changed();
    }

    // the simulator takes device `dev` as the project has it now, without a restart
    fn keep(&mut self, dev: u32) {
        if let Some(d) = self.proj.device(dev) {
            self.sim.keep(d);
        }
    }

    pub(crate) fn undo(&mut self) -> bool {
        let kept = Kept::dev_of(self.hist.undo_value());
        let done = self.hist.undo(&mut self.proj);
        if done {
            if let Some(d) = kept {
                self.keep(d);
            }
            self.changed();
        }
        done
    }

    pub(crate) fn redo(&mut self) -> bool {
        let kept = Kept::dev_of(self.hist.redo_value());
        let done = self.hist.redo(&mut self.proj);
        if done {
            if let Some(d) = kept {
                self.keep(d);
            }
            self.changed();
        }
        done
    }

    pub(crate) fn dirty(&self) -> bool {
        self.hist.dirty()
    }

    /// the project was written to path
    pub(crate) fn saved(&mut self, path: &str) {
        self.path = Some(String::from(path));
        self.hist.mark_saved();
    }

    /// the name of the file ("Büro.jfnet"), a lab's file name, "Unbenannt" without one
    pub(crate) fn file_name(&self) -> String {
        match (&self.path, &self.origin) {
            (Some(p), _) => String::from(p.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(p)),
            (None, Some(o)) => o.clone(),
            (None, None) => String::from(crate::app::tr::t("Unbenannt")),
        }
    }
}
