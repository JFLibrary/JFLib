// data_link -- (JD18, RD-09, AS-04, DA-04) Daten verknüpfen: columns against shape fields,
// for the page or the selection
//
// STUB (skeleton): the refresh and link agent fills it in.

use ui::Ui;

use crate::Diagrammer;

/// the wizard's state
#[derive(Default)]
pub(crate) struct LinkState {}

/// Daten › Daten verknüpfen
pub(crate) fn open(_app: &mut Diagrammer, _ui: &mut Ui) {}

/// every pass: the wizard when open
pub(crate) fn dialog(_app: &mut Diagrammer, _ui: &mut Ui) {}
