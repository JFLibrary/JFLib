// data_textdraw -- (JD18, IM-08, AS-20) Zeichnung aus Text: a CSV or text file with Shape and
// Link records becomes shapes and connectors, with a preview and the choice of masters
//
// STUB (skeleton): the text drawing agent fills it in.

use ui::Ui;

use crate::Diagrammer;

/// the wizard's state
#[derive(Default)]
pub(crate) struct TextDrawState {}

/// Daten › Zeichnung aus Text
pub(crate) fn open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// every pass: the wizard when open
pub(crate) fn dialog(_app: &mut Diagrammer, _ui: &mut Ui) {}
