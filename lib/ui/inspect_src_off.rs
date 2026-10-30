// inspect_src_off -- the inspector of a --release build shows no sources: the library's widget
// calls carry #[track_caller] only in development builds (inspect_src.rs; build_rs.py passes
// --cfg jfgui_dev there), their scopes are empty here and fold away, nothing is stored.  The
// script commands that need sources fail with the reason.

use super::auto::word_eq;
use super::inspect::Src;
use super::{App, Ui};

/// the inspector shows no sources (a --release build)
pub(crate) const SRC: bool = false;

/// (ui.m.isrc) nothing to keep
#[derive(Default)]
pub(crate) struct S;

/// a widget call's scope: nothing
pub(crate) struct SrcScope;

impl Ui {
    #[inline(always)]
    pub(crate) fn src_scope(&self) -> SrcScope {
        SrcScope
    }

    #[inline(always)]
    pub(crate) fn src_sem(&mut self, _k: usize) {}

    #[inline(always)]
    pub(crate) fn src_cur(&self) -> Option<Src> {
        None
    }

    pub(crate) fn src_of(&self, _k: usize) -> Option<Src> {
        None
    }

    pub(crate) fn src_clear(&mut self) {}

    pub(crate) fn src_count(&self) -> usize {
        0
    }

    pub(crate) fn isrc_props(&mut self, _src: Option<Src>) {}

    /// `inspect editor|open ...`: refused
    pub(crate) fn isrc_script(&mut self, _app: &mut dyn App, a: &[u8]) -> bool {
        if word_eq(a, "editor").is_some() || word_eq(a, "open").is_some() {
            self.fail(b"the sources of widgets need a development build (build_rs.py without --release)");
            return true;
        }
        false
    }

    /// `inspect expect source|sources|opened|note ...`: refused
    pub(crate) fn isrc_expect(&mut self, a: &[u8]) -> bool {
        if ["source", "sources", "opened", "note"].iter().any(|c| word_eq(a, c).is_some()) {
            self.fail(b"the sources of widgets need a development build (build_rs.py without --release)");
            return true;
        }
        false
    }
}
