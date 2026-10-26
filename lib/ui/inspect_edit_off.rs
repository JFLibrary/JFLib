// inspect_edit_off -- the inspector of a --release build reads only: editing in it is a
// development build's (inspect_edit.rs; build_rs.py passes --cfg jfgui_dev there).  The hooks
// of inspect.rs do nothing here; the script commands that edit fail with the reason.

use super::auto::word_eq;
use super::{App, Sem, Ui};

/// the inspector cannot edit (a --release build)
pub(crate) const EDIT: bool = false;

/// (ui.m.ied) nothing to keep
#[derive(Default)]
pub(crate) struct S;

impl Ui {
    /// (inspect.rs, tab "Farben") false: the read-only list is drawn
    pub(crate) fn ied_colors(&mut self) -> bool {
        false
    }

    pub(crate) fn ied_look(&mut self) {}

    pub(crate) fn ied_operate(&mut self, _r: &Sem) {}

    pub(crate) fn ied_keys(&mut self) {}

    /// (the tool window keeps the app's look)
    pub(crate) fn ied_plain(&mut self, _on: bool) {}

    pub(crate) fn ied_after(&mut self) {}

    /// `inspect set|reset|undo|redo|save|list|do ...`: refused
    pub(crate) fn ied_script(&mut self, _app: &mut dyn App, a: &[u8]) -> bool {
        if ["set", "reset", "undo", "redo", "save", "list", "do"].iter().any(|c| word_eq(a, c).is_some()) {
            self.fail(b"editing in the inspector needs a development build (build_rs.py without --release)");
            return true;
        }
        false
    }

    pub(crate) fn ied_expect(&mut self, _a: &[u8]) -> bool {
        false
    }
}
