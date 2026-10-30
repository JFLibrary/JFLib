// groups -- the groups and the background picture of the logical canvas as undo steps (G1)
//
//   step(doc, label, f) -> Result<Option<R>, GroupErr>   runs f (one of jfn_model::group's
//                                   edits) on the project; what it changed in the groups becomes
//                                   one undo step (None: nothing changed, no step)
//   set_bg(doc, label, bg) -> bool  the picture in, replaced or out (one step; undo brings the
//                                   one before back, the bytes are shared, not copied)
//   bg_place(doc, label, f) -> bool its place, scale, opacity or lock (one step without the
//                                   bytes)
//   bg_live(doc, f) / bg_since(doc, before, label)   a drag or resize on the canvas: f changes
//                                   the place at once (no step), the step from `before`
//                                   (Background::place) comes when it ends
//   names()                         the word new groups are named with in the window's
//                                   language ("Gruppe 1")
//
// Groups and the picture are view data: the simulator follows nothing here (Doc::record's sync
// finds no difference), so the engine, its keyframes and every report stay as they are.  A
// group drag moves devices and notes: those are edits::moves steps, as for any drag.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::group::{Group, GroupErr};
use jfn_model::{Background, Project};
use ui::Edit;

use super::Doc;
use crate::app::tr::t;

/// the groups before and after an edit
struct Step {
    from: Vec<Group>,
    to: Vec<Group>,
    label: String,
}

impl Edit<Project> for Step {
    fn apply(&mut self, p: &mut Project) {
        p.groups = self.to.clone();
    }
    fn revert(&mut self, p: &mut Project) {
        p.groups = self.from.clone();
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// the word new groups are named with ("Gruppe 1")
pub(crate) fn names() -> &'static str {
    t("Gruppe")
}

/// one edit of the groups as an undo step (applied); Ok(None): nothing changed
pub(crate) fn step<R>(doc: &mut Doc, label: &str, f: impl FnOnce(&mut Project) -> Result<R, GroupErr>) -> Result<Option<R>, GroupErr> {
    let before = doc.proj.groups.clone();
    let r = f(&mut doc.proj);
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            doc.proj.groups = before;
            return Err(e);
        }
    };
    if doc.proj.groups == before {
        return Ok(None);
    }
    let to = doc.proj.groups.clone();
    doc.record(Box::new(Step { from: before, to, label: String::from(label) }));
    Ok(Some(r))
}

/// the picture before and after (in, replaced, out)
struct BgSet {
    from: Option<Background>,
    to: Option<Background>,
    label: String,
}

impl Edit<Project> for BgSet {
    fn apply(&mut self, p: &mut Project) {
        p.background = self.to.clone();
    }
    fn revert(&mut self, p: &mut Project) {
        p.background = self.from.clone();
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// the picture's place, scale, opacity and lock before and after
struct BgPlace {
    from: [i64; 5],
    to: [i64; 5],
    label: String,
}

impl Edit<Project> for BgPlace {
    fn apply(&mut self, p: &mut Project) {
        if let Some(b) = p.background.as_mut() {
            b.set_place(self.to);
        }
    }
    fn revert(&mut self, p: &mut Project) {
        if let Some(b) = p.background.as_mut() {
            b.set_place(self.from);
        }
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// the picture in, replaced or out (one step) -> it changed
pub(crate) fn set_bg(doc: &mut Doc, label: &str, bg: Option<Background>) -> bool {
    if doc.proj.background == bg {
        return false;
    }
    let from = doc.proj.background.clone();
    doc.apply(Box::new(BgSet { from, to: bg, label: String::from(label) }));
    true
}

/// a change of the picture's place, scale, opacity or lock (one step) -> it changed
pub(crate) fn bg_place(doc: &mut Doc, label: &str, f: impl FnOnce(&mut Background)) -> bool {
    let Some(b) = doc.proj.background.as_ref() else { return false };
    let from = b.place();
    let mut c = b.clone();
    f(&mut c);
    let to = c.place();
    if to == from {
        return false;
    }
    doc.apply(Box::new(BgPlace { from, to, label: String::from(label) }));
    true
}

/// a drag or resize in progress changes the picture at once (no step)
pub(crate) fn bg_live(doc: &mut Doc, f: impl FnOnce(&mut Background)) {
    if let Some(b) = doc.proj.background.as_mut() {
        f(b);
        doc.touch();
    }
}

/// the step of a drag that changed the picture live since `before` (false: it is the same)
pub(crate) fn bg_since(doc: &mut Doc, before: [i64; 5], label: &str) -> bool {
    let Some(to) = doc.proj.background.as_ref().map(|b| b.place()) else { return false };
    if to == before {
        return false;
    }
    doc.record(Box::new(BgPlace { from: before, to, label: String::from(label) }));
    true
}
