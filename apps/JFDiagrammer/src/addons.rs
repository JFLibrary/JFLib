// addons -- (JD21) the tools in place of Visio's add-ons (Ansicht › Add-Ons: Shapes nummerieren,
// Array, Maßlinien; AS-11, RA-09) and the container styles' galleries and categories (ST-02)
//
//   Addons               the state of the dialogs and galleries
//   available(app, id)   whether one of these commands can run now (commands::usable asks)
//   dialogs(app, ui)     every pass: the dialogs
//   after(app, ui)       every pass after the ribbon: the galleries' live preview
//
//   dlg_number.rs        Shapes nummerieren, Beim Ablegen weiterzählen
//   dlg_array.rs         Array
//   tools_dim.rs         Maßlinie horizontal / vertikal / ausgerichtet (also as a tool)
//   container_styles.rs  Containerformatvorlagen, Kopfzeilenstil and Einfügen › Container as
//                        galleries with a live preview
//   dlg_categories.rs    Kategorien (containers' required and excluded ones, shapes' own)

use ui::*;

use crate::Diagrammer;

/// the state of the add-ons' dialogs and the container styles' galleries
#[derive(Default)]
pub(crate) struct Addons {
    pub number: crate::dlg_number::NumberDialog,
    pub array: crate::dlg_array::ArrayDialog,
    pub cats: crate::dlg_categories::CatDialog,
    pub styles: crate::container_styles::Styles,
}

/// whether one of these commands can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    crate::dlg_number::available(app, id)
        && crate::dlg_array::available(app, id)
        && crate::tools_dim::available(app, id)
        && crate::dlg_categories::available(app, id)
        && crate::container_styles::available(app, id)
}

/// every pass: the dialogs
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    crate::dlg_number::dialog(app, ui);
    crate::dlg_array::dialog(app, ui);
    crate::dlg_categories::dialog(app, ui);
}

/// every pass after the ribbon: a style's preview goes when its tile is left
pub(crate) fn after(app: &mut Diagrammer, ui: &mut Ui) {
    crate::container_styles::after(app, ui);
}
