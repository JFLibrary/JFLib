// Native DLL/SO plugins cannot be loaded by a browser. Built-in effects and codecs
// still register through their ordinary library entrypoints and keep their ordering.
use crate::app::Pn;
use crate::app::constants::*;
use crate::app::commands::pn_cmd_set;
use alloc::string::String;
use ui::*;

#[derive(Default)]
pub(crate) struct Plug;

pub(crate) fn plug_init(_p: &mut Pn, _ui: &mut Ui) {
    ::fx::use_all();
    img_all::use_all_codecs();
}
pub(crate) fn plug_states(_p: &mut Pn, ui: &mut Ui) {
    pn_cmd_set(ui, CMD_FX_PLUGIN_ERRORS, CF_DISABLED);
}
pub(crate) fn plug_frame(_p: &mut Pn, ui: &mut Ui) {
    ui.last_reset();
    ui.sem(
        K_TEXT,
        ID_PLUG_FIRST,
        Some("Native DLL-/SO-Plugins sind im Browser nicht verfügbar."),
        0,
    );
}
pub(crate) fn plug_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id == CMD_FX_PLUGIN_ERRORS {
        crate::workspace::dialogs::settings::set_cmd(p, ui, CMD_SET_PAGE, SP_PLUGINS as u32);
        return true;
    }
    if (CMD_P1_FIRST..=CMD_P1_LAST).contains(&id) {
        return false;
    }
    // Statically registered library effects can use dynamically populated menu rows.
    crate::effects::fx_cmd(p, ui, id, arg)
}
pub(crate) fn plug_shapes_feed(_p: &mut Pn) {}
pub(crate) fn plug_count(_p: &Plug) -> i32 {
    0
}
pub(crate) fn plug_errors(_p: &Plug) -> i32 {
    0
}
pub(crate) fn plug_loaded(_p: &Plug) -> i32 {
    0
}
pub(crate) fn plug_text(_p: &Plug, _index: i32) -> (u32, String) {
    (0, String::new())
}
