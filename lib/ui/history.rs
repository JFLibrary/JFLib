// history -- undo / redo for an app's own state
//
//   History::new(limit)                  empty; at most `limit` steps are kept (the oldest go)
//   h.apply(&mut state, edit)            run an Edit and record it (the redo steps go)
//   h.undo(&mut state) / h.redo(&mut state) -> bool
//   h.can_undo() / can_redo()            undo_label() / redo_label() -> the step's label;
//                                        undo_value() / redo_value() -> the step as data
//   h.group_begin(label) / group_end()   the edits between them are one step
//   h.mark_saved() / dirty()             whether the state differs from the saved one
//   h.clear()
//   Snapshot::new(label, before, after)  an Edit for small states: the whole value (Clone)
//   ui.undo_keys() -> Option<bool>       Ctrl+Z (Some(true)) / Ctrl+Y or Ctrl+Shift+Z
//                                        (Some(false)) when no text field has the keyboard;
//                                        the key is consumed
//
// An Edit changes the state forward (apply) and back (revert); merge lets a step absorb the
// next one of the same kind (typing in one field, dragging a slider) so one undo takes it
// all back.  Text fields keep their own undo; the edits here are the app's.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::Ui;
use super::consts::*;

/// one undoable change of a state S
pub trait Edit<S> {
    /// do it (again)
    fn apply(&mut self, s: &mut S);
    /// take it back
    fn revert(&mut self, s: &mut S);
    /// shown in "Rückgängig: …"
    fn label(&self) -> &str;
    /// absorb the following edit into this one (true: it is part of this step now)
    fn merge(&mut self, _next: &mut dyn Edit<S>) -> bool {
        false
    }
    /// (merge) the edit as Any-like data: the merge partner's own value
    fn value(&self) -> Option<&dyn core::any::Any> {
        None
    }
}

/// an Edit of a whole small value (Clone): before and after
pub struct Snapshot<T: Clone + 'static> {
    label: String,
    before: T,
    after: T,
    /// merge key: snapshots with the same non-zero key merge (one step for a drag)
    key: u32,
}

impl<T: Clone + 'static> Snapshot<T> {
    pub fn new(label: &str, before: T, after: T) -> Self {
        Snapshot { label: String::from(label), before, after, key: 0 }
    }
    /// edits with the same key merge into one step
    pub fn merging(mut self, key: u32) -> Self {
        self.key = key;
        self
    }
}

impl<T: Clone + 'static> Edit<T> for Snapshot<T> {
    fn apply(&mut self, s: &mut T) {
        *s = self.after.clone();
    }
    fn revert(&mut self, s: &mut T) {
        *s = self.before.clone();
    }
    fn label(&self) -> &str {
        &self.label
    }
    fn merge(&mut self, next: &mut dyn Edit<T>) -> bool {
        if self.key == 0 {
            return false;
        }
        match next.value().and_then(|v| v.downcast_ref::<Snapshot<T>>()) {
            Some(n) if n.key == self.key => {
                self.after = n.after.clone();
                true
            }
            _ => false,
        }
    }
    fn value(&self) -> Option<&dyn core::any::Any> {
        Some(self)
    }
}

// several edits as one step
struct Group<S> {
    label: String,
    edits: Vec<Box<dyn Edit<S>>>,
}

impl<S: 'static> Edit<S> for Group<S> {
    fn apply(&mut self, s: &mut S) {
        for e in self.edits.iter_mut() {
            e.apply(s);
        }
    }
    fn revert(&mut self, s: &mut S) {
        for e in self.edits.iter_mut().rev() {
            e.revert(s);
        }
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// the undo and redo steps of a state S
pub struct History<S> {
    done: Vec<Box<dyn Edit<S>>>,
    undone: Vec<Box<dyn Edit<S>>>,
    limit: usize,
    /// open groups (label, edits so far)
    groups: Vec<(String, Vec<Box<dyn Edit<S>>>)>,
    /// done.len() at the last save (None: the saved state is gone)
    saved: Option<usize>,
}

impl<S: 'static> History<S> {
    pub fn new(limit: usize) -> Self {
        History { done: Vec::new(), undone: Vec::new(), limit: limit.max(1), groups: Vec::new(), saved: Some(0) }
    }

    /// run the edit and record it as the newest step (redo steps are gone)
    pub fn apply(&mut self, s: &mut S, mut edit: Box<dyn Edit<S>>) {
        edit.apply(s);
        self.record(edit);
    }

    /// record an edit that was already applied
    pub fn record(&mut self, mut edit: Box<dyn Edit<S>>) {
        if let Some(g) = self.groups.last_mut() {
            g.1.push(edit);
            return;
        }
        self.undone.clear();
        if self.saved.is_some_and(|k| k > self.done.len()) {
            self.saved = None; // the saved state was in the redo steps
        }
        // merge into the newest step (not across the saved point)
        let at_save = self.saved == Some(self.done.len());
        if let Some(last) = self.done.last_mut() {
            if !at_save && last.merge(edit.as_mut()) {
                return;
            }
        }
        self.done.push(edit);
        if self.done.len() > self.limit {
            self.done.remove(0);
            self.saved = match self.saved {
                Some(0) => None,
                Some(k) => Some(k - 1),
                None => None,
            };
        }
    }

    /// take the newest step back -> false when there is none
    pub fn undo(&mut self, s: &mut S) -> bool {
        let Some(mut e) = self.done.pop() else { return false };
        e.revert(s);
        self.undone.push(e);
        true
    }

    /// do the last undone step again -> false when there is none
    pub fn redo(&mut self, s: &mut S) -> bool {
        let Some(mut e) = self.undone.pop() else { return false };
        e.apply(s);
        self.done.push(e);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
    /// the label of the step undo would take back
    pub fn undo_label(&self) -> Option<&str> {
        self.done.last().map(|e| e.label())
    }
    /// the label of the step redo would do again
    pub fn redo_label(&self) -> Option<&str> {
        self.undone.last().map(|e| e.label())
    }
    /// the step undo would take back as data (Edit::value; None for a group): the app can
    /// see what kind of edit it is before it runs
    pub fn undo_value(&self) -> Option<&dyn core::any::Any> {
        self.done.last().and_then(|e| e.value())
    }
    /// the step redo would do again as data (Edit::value)
    pub fn redo_value(&self) -> Option<&dyn core::any::Any> {
        self.undone.last().and_then(|e| e.value())
    }

    /// the following edits are one step (nested groups join the outer one)
    pub fn group_begin(&mut self, label: &str) {
        self.groups.push((String::from(label), Vec::new()));
    }

    /// close the group of group_begin (an empty group records nothing)
    pub fn group_end(&mut self) {
        let Some((label, edits)) = self.groups.pop() else { return };
        if !edits.is_empty() {
            self.record(Box::new(Group { label, edits }));
        }
    }

    /// the state is saved now (dirty() false until the next change)
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.done.len());
    }

    /// the state differs from the saved one
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.done.len())
    }

    /// forget every step (the current state counts as saved when it was)
    pub fn clear(&mut self) {
        let dirty = self.dirty();
        self.done.clear();
        self.undone.clear();
        self.groups.clear();
        self.saved = if dirty { None } else { Some(0) };
    }

    /// the number of undo steps
    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }
}

impl Ui {
    /// undo_keys() -> Some(true) for Ctrl+Z, Some(false) for Ctrl+Y / Ctrl+Shift+Z, when no
    /// text field has the keyboard (they keep their own undo); the key is consumed
    pub fn undo_keys(&mut self) -> Option<bool> {
        // (called before the widgets of the pass: the last pass knows the focused one)
        if self.focus_is_text() || self.focus_was_text() {
            return None;
        }
        let k = self.key_first();
        let (vk, m) = (k & 0xFFFF, k >> 16);
        if m & (MOD_CTRL | MOD_ALT) != MOD_CTRL {
            return None;
        }
        let shift = m & MOD_SHIFT != 0;
        let r = match vk {
            0x5A if !shift => Some(true), // Z
            0x5A => Some(false),          // Shift+Z
            0x59 if !shift => Some(false), // Y
            _ => None,
        };
        if r.is_some() {
            self.key(vk);
        }
        r
    }
}
