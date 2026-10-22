// faults -- the lab's faults as steps of the undo history: injecting and healing change nothing
// in the project, the simulator follows them through jfn_sim's fault actions
//
//   doc.inject(fault, dur) -> Result<id, Msg>   a fault starts now (Action::Fault), one undo
//                                               step; the engine's number of the fault
//   doc.heal(&[id]) -> Result<n, Msg>           active faults end now (Action::Heal each), one
//                                               undo step; how many ended
//   doc.heal_all() -> Result<n, Msg>            every active fault ends (Action::HealAll), one
//                                               undo step
//   Step::of(history value)                     a step of faults (Doc::undo / redo look for
//                                               one, as they do for a kept configuration)
//   Runs                                        the faults the window knows: a key of its own
//                                               per fault and the engine's number and start of
//                                               its current run
//
// The consistent way with the other edits: the undo history is the project's, and the engine
// follows it through actions.  A fault is no part of the project, so its step leaves the
// project as it is (apply and revert do nothing) and Doc runs the actions itself when the step
// is done, undone or redone (Doc::fault_step):
//
//   undo an injection   its fault heals now
//   redo an injection   the fault starts again now with its whole duration
//   undo a heal         the faults start again with the time they had left when they healed
//   redo a heal         they heal again
//
// Every run is a fault of its own in the engine (a new number, a new mark on the timeline: the
// charts show each run), the key ties the runs of one fault together.  A run is found again by
// its number, start and fault: a fresh engine (Simulation zurücksetzen) numbers from 1 again,
// and an old step must not heal a new fault.  A fault that healed by itself or was healed
// otherwise is left alone.  Faults are not saved with the project: a step leaves the document
// as clean, or as changed, as it was (History::mark_saved when it was clean).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::{Msg, Project};
use jfn_sim::{Action, Cmd, Fault, FaultMark, Ns};
use ui::Edit;

use super::sim::Sim;
use super::Doc;

/// one fault of a step: its key, what it does, how long it lasts (None: until it is healed)
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub key: u32,
    pub fault: Fault,
    pub dur: Option<Ns>,
}

/// a step of the history: faults that start (start: true) or end when it is done
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub label: String,
    pub start: bool,
    pub items: Vec<Item>,
}

impl Step {
    /// the step of faults behind a value of the history (History::undo_value / redo_value)
    pub(crate) fn of(v: Option<&dyn core::any::Any>) -> Option<Step> {
        v.and_then(|v| v.downcast_ref::<Step>()).cloned()
    }
}

impl Edit<Project> for Step {
    fn apply(&mut self, _: &mut Project) {}
    fn revert(&mut self, _: &mut Project) {}
    fn label(&self) -> &str {
        &self.label
    }
    fn value(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

/// the current run of a fault: the engine's number and when it started
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    id: u32,
    at: Ns,
}

/// the faults the window knows (key -> current run)
#[derive(Default)]
pub(crate) struct Runs {
    runs: BTreeMap<u32, Run>,
    last: u32,
}

impl Runs {
    fn new_key(&mut self) -> u32 {
        self.last += 1;
        self.last
    }

    /// the fault of an item starts now -> its number
    fn start(&mut self, sim: &mut Sim, it: &Item) -> Result<u32, Msg> {
        let id = sim.engine().next_fault_id();
        sim.send(Cmd::Apply(Action::Fault { fault: it.fault.clone(), dur: it.dur })).map_err(|e| e.msg())?;
        let at = sim.engine().faults().iter().find(|m| m.id == id).map_or(0, |m| m.at);
        self.runs.insert(it.key, Run { id, at });
        Ok(id)
    }

    /// the current run of an item, when the engine has it active
    fn active(&self, sim: &Sim, it: &Item) -> Option<u32> {
        let r = self.runs.get(&it.key)?;
        sim.engine().active_faults().any(|m| m.id == r.id && m.at == r.at && m.fault == it.fault).then_some(r.id)
    }

    /// the item's fault heals now (when its run is active)
    fn heal(&mut self, sim: &mut Sim, it: &Item) {
        if let Some(id) = self.active(sim, it) {
            let _ = sim.send(Cmd::Apply(Action::Heal { id }));
        }
    }

    /// the key of a mark (a new one for a fault the window did not start)
    fn key_of(&mut self, m: &FaultMark) -> u32 {
        if let Some((&k, _)) = self.runs.iter().find(|(_, r)| r.id == m.id && r.at == m.at) {
            return k;
        }
        let k = self.new_key();
        self.runs.insert(k, Run { id: m.id, at: m.at });
        k
    }
}

impl Doc {
    // a step of faults into the history: the document stays as clean as it was
    fn push_fault(&mut self, s: Step) {
        let clean = !self.hist.dirty();
        self.hist.record(Box::new(s));
        if clean {
            self.hist.mark_saved();
        }
    }

    /// a fault starts now (one undo step) -> the engine's number of it
    pub(crate) fn inject(&mut self, fault: Fault, dur: Option<Ns>) -> Result<u32, Msg> {
        let key = self.runs.new_key();
        let item = Item { key, fault, dur };
        let id = self.runs.start(&mut self.sim, &item)?;
        self.push_fault(Step { label: String::from("Fehler einspielen"), start: true, items: alloc::vec![item] });
        Ok(id)
    }

    // the items of active marks: each with the time it has left
    fn items_of(&mut self, marks: &[FaultMark]) -> Vec<Item> {
        let now = self.sim.engine().now();
        marks.iter().map(|m| Item { key: self.runs.key_of(m), fault: m.fault.clone(), dur: m.until.map(|u| u.saturating_sub(now).max(1)) }).collect()
    }

    /// the active faults `ids` end now (one undo step) -> how many ended
    pub(crate) fn heal(&mut self, ids: &[u32]) -> Result<usize, Msg> {
        let marks: Vec<FaultMark> = self.sim.engine().active_faults().filter(|m| ids.contains(&m.id)).cloned().collect();
        if marks.is_empty() {
            return Ok(0);
        }
        let items = self.items_of(&marks);
        for m in &marks {
            self.sim.send(Cmd::Apply(Action::Heal { id: m.id })).map_err(|e| e.msg())?;
        }
        let label = if marks.len() == 1 { "Fehler beheben" } else { "Fehler beheben (mehrere)" };
        self.push_fault(Step { label: String::from(label), start: false, items });
        Ok(marks.len())
    }

    /// every active fault ends now (one undo step) -> how many ended
    pub(crate) fn heal_all(&mut self) -> Result<usize, Msg> {
        let marks: Vec<FaultMark> = self.sim.engine().active_faults().cloned().collect();
        if marks.is_empty() {
            return Ok(0);
        }
        let items = self.items_of(&marks);
        self.sim.send(Cmd::Apply(Action::HealAll)).map_err(|e| e.msg())?;
        self.push_fault(Step { label: String::from("Alle Fehler beheben"), start: false, items });
        Ok(marks.len())
    }

    /// undo (back) or redo a step of faults: the history moves, the engine follows
    pub(crate) fn fault_step(&mut self, s: Step, back: bool) -> bool {
        let clean = !self.hist.dirty();
        let done = if back { self.hist.undo(&mut self.proj) } else { self.hist.redo(&mut self.proj) };
        if !done {
            return false;
        }
        // an injection undone and a heal redone end the faults; the others start them again
        let start = s.start != back;
        for it in &s.items {
            if start {
                let _ = self.runs.start(&mut self.sim, it);
            } else {
                self.runs.heal(&mut self.sim, it);
            }
        }
        if clean {
            self.hist.mark_saved();
        }
        true
    }

    /// the engine's number of the current run of a step's item, when it is active (tests)
    pub(crate) fn fault_run(&self, it: &Item) -> Option<u32> {
        self.runs.active(&self.sim, it)
    }
}
