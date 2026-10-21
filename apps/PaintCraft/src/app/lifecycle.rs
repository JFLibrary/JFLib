//! Startup order, frame orchestration and command routing.
use super::{Pn, constants::*};
use ui::*;

type ModFn = fn(&mut Pn, &mut Ui);
type CmdFn = fn(&mut Pn, &mut Ui, u32, u32) -> bool;

/// mod_init: every module's init, in this order
static MOD_INIT: [ModFn; 29] = [
    crate::workspace::icons::pn_icons_init,
    crate::document::model::doc_init,
    crate::integrations::plugins::plug_init,
    crate::integrations::paint_dot_net::catalog::init,
    crate::integrations::file_io::fio_init,
    crate::document::history::hist_modinit,
    crate::document::clipboard::clip_init,
    crate::document::selection::sel_init,
    crate::workspace::palette::pal_init,
    crate::workspace::view::view_init,
    crate::document::operations::iop_init,
    crate::document::layers::lay_init,
    crate::effects::fx_init,
    crate::tools::tool_init,
    crate::workspace::tabs::tabs_init,
    crate::workspace::chrome::chrome_init,
    crate::workspace::dialogs::settings::set_init,
    crate::preferences::localization::init,
    crate::diagnostics::crash::crash_setup,
    crate::workspace::dialogs::misc::misc_init,
    crate::diagnostics::diag_init,
    crate::workspace::panels::tools::wtools_init,
    crate::workspace::panels::history::whist_init,
    crate::workspace::panels::layers::wlay_init,
    crate::workspace::panels::colors::wcol_init,
    crate::tools::gradient::tgrad_init,
    crate::tools::shapes::tshp_init,
    crate::tools::text::ttext_init,
    crate::tools::clone::tclone_init,
];

/// mod_states
static MOD_STATES: [ModFn; 20] = [
    crate::integrations::paint_dot_net::catalog::states,
    crate::integrations::services::states,
    crate::integrations::paint_dot_net::host::states,
    crate::integrations::gmic::states,
    crate::integrations::file_io::fio_states,
    crate::document::history::hist_states,
    crate::document::clipboard::clip_states,
    crate::document::selection::sel_states,
    crate::workspace::palette::pal_states,
    crate::workspace::view::view_states,
    crate::document::operations::iop_states,
    crate::document::layers::lay_states,
    crate::effects::fx_states,
    crate::tools::tool_states,
    crate::workspace::tabs::tabs_states,
    crate::workspace::chrome::chrome_states,
    crate::workspace::dialogs::settings::set_states,
    crate::workspace::dialogs::misc::misc_states,
    crate::diagnostics::diag_states,
    crate::integrations::plugins::plug_states,
];

/// mod_frame
static MOD_FRAME: [ModFn; 22] = [
    crate::integrations::services::frame,
    crate::integrations::paint_dot_net::host::frame,
    crate::integrations::gmic::frame,
    crate::tools::brushes::custom::frame,
    crate::workspace::panels::dock::dock_frame,
    crate::workspace::panels::tools::wtools_frame,
    crate::workspace::panels::history::whist_frame,
    crate::workspace::panels::layers::wlay_frame,
    crate::workspace::panels::colors::wcol_frame,
    crate::integrations::file_io::fio_frame,
    crate::document::history::hist_frame,
    crate::document::clipboard::clip_frame,
    crate::document::selection::sel_frame,
    crate::workspace::palette::pal_frame,
    crate::document::operations::iop_frame,
    crate::document::layers::lay_frame,
    crate::effects::fx_frame,
    crate::tools::tool_frame,
    crate::workspace::dialogs::settings::set_frame,
    crate::workspace::dialogs::misc::misc_frame,
    crate::diagnostics::diag_frame,
    crate::integrations::plugins::plug_frame,
];

/// mod_cmd: broadcast order for UIC_* (the tool gets Esc first)
static MOD_CMD: [CmdFn; 15] = [
    crate::tools::tool_cmd,
    crate::workspace::view::view_cmd,
    crate::document::selection::sel_cmd,
    crate::document::history::hist_cmd,
    crate::document::clipboard::clip_cmd,
    crate::integrations::file_io::fio_cmd,
    crate::document::operations::iop_cmd,
    crate::document::layers::lay_cmd,
    crate::effects::fx_cmd,
    crate::workspace::palette::pal_cmd,
    crate::workspace::tabs::tabs_cmd,
    crate::workspace::chrome::chrome_cmd,
    crate::workspace::dialogs::settings::set_cmd,
    crate::workspace::dialogs::misc::misc_cmd,
    crate::diagnostics::diag_cmd,
];

/// route_tab: command ranges (lo, hi, handler); ids outside every range are not handled
static ROUTE_TAB: [(u32, u32, CmdFn); 30] = [
    (
        crate::integrations::paint_dot_net::catalog::FIRST,
        crate::integrations::paint_dot_net::catalog::LAST,
        crate::integrations::paint_dot_net::catalog::command,
    ),
    (
        CMD_ACQUIRE,
        CMD_ACQUIRE,
        crate::integrations::services::command,
    ),
    (
        crate::integrations::gmic::COMMAND,
        crate::integrations::gmic::COMMAND,
        crate::integrations::gmic::command,
    ),
    (
        crate::integrations::paint_dot_net::host::COMMAND,
        crate::integrations::paint_dot_net::host::COMMAND,
        crate::integrations::paint_dot_net::host::command,
    ),
    // P1 (plugins.rs): test hooks, Plugin-Fehler…, plugin menu rows (before fio / fx)
    (
        CMD_P1_FIRST,
        CMD_P1_LAST,
        crate::integrations::plugins::plug_cmd,
    ),
    (
        CMD_FX_PLUGIN_ERRORS,
        CMD_FX_PLUGIN_ERRORS,
        crate::integrations::plugins::plug_cmd,
    ),
    (
        CMD_ADJ_PLUGIN0,
        CMD_ADJ_LAST,
        crate::integrations::plugins::plug_cmd,
    ),
    (
        CMD_FX_PLUGIN0,
        CMD_FX_LAST,
        crate::integrations::plugins::plug_cmd,
    ),
    (
        CMD_FILE_FIRST,
        CMD_FILE_LAST,
        crate::integrations::file_io::fio_cmd,
    ),
    (
        CMD_HIST_FIRST,
        CMD_HIST_LAST,
        crate::document::history::hist_cmd,
    ),
    (
        CMD_CLIP_FIRST,
        CMD_CLIP_LAST,
        crate::document::clipboard::clip_cmd,
    ),
    (
        CMD_SEL_FIRST,
        CMD_SEL_LAST,
        crate::document::selection::sel_cmd,
    ),
    (
        CMD_PAL_FIRST,
        CMD_PAL_LAST,
        crate::workspace::palette::pal_cmd,
    ),
    (
        CMD_VIEW_FIRST,
        CMD_VIEW_LAST,
        crate::workspace::view::view_cmd,
    ),
    (
        CMD_IMG_FIRST,
        CMD_IMG_LAST,
        crate::document::operations::iop_cmd,
    ),
    (
        CMD_LAYER_FIRST,
        CMD_LAYER_LAST,
        crate::document::layers::lay_cmd,
    ),
    (CMD_ADJ_FIRST, CMD_FX_LAST, crate::effects::fx_cmd),
    (
        CMD_WND_TOOLS,
        CMD_WND_TRANSLUCENT,
        crate::workspace::chrome::chrome_cmd,
    ),
    (
        CMD_IMG_NEXT,
        CMD_IMG_LIST + 1,
        crate::workspace::tabs::tabs_cmd,
    ),
    (
        CMD_SETTINGS,
        CMD_SETTINGS,
        crate::workspace::dialogs::settings::set_cmd,
    ),
    (
        CMD_HELP,
        CMD_HELP_DOCS - 1,
        crate::workspace::chrome::chrome_cmd,
    ),
    (
        CMD_HELP_DOCS,
        CMD_WND_LAST,
        crate::workspace::dialogs::misc::misc_cmd,
    ),
    (
        CMD_TGRAD_FIRST,
        CMD_TGRAD_LAST,
        crate::tools::gradient::tgrad_cmd,
    ),
    (
        CMD_TSHP_FIRST,
        CMD_TSHP_LAST,
        crate::tools::shapes::tshp_cmd,
    ),
    (CMD_TOOL_FIRST, CMD_TOOL_LAST, crate::tools::tool_cmd),
    (
        CMD_TTEXT_FIRST,
        CMD_TTEXT_LAST,
        crate::tools::text::ttext_cmd,
    ),
    (
        CMD_TCLONE_FIRST,
        CMD_TCLONE_LAST,
        crate::tools::clone::tclone_cmd,
    ),
    (
        CMD_TAB_FIRST,
        CMD_TAB_LAST,
        crate::workspace::tabs::tabs_cmd,
    ),
    (
        CMD_SET_FIRST,
        CMD_SET_LAST,
        crate::workspace::dialogs::settings::set_cmd,
    ),
    (CMD_TEST_FIRST, CMD_TEST_LAST, crate::diagnostics::diag_cmd),
];

/// pn_init() -- every module's init (crash report mode: none, crash.rs shows only its window)
pub(super) fn pn_init(p: &mut Pn, ui: &mut Ui) {
    if crate::diagnostics::crash::crash_mode_init(p, ui) {
        return;
    }
    for f in MOD_INIT.iter() {
        f(p, ui);
    }
}

/// pn_frame() -- the main window, then the modules' windows and dialogs
pub(super) fn pn_frame(p: &mut Pn, ui: &mut Ui) {
    if crate::diagnostics::crash::crash_mode(&p.crash) {
        crate::diagnostics::crash::crash_frame(p, ui);
        return;
    }
    crate::workspace::icons::pn_icons_colors(p, ui); // the render-time icon hook compares with them
    for f in MOD_STATES.iter() {
        f(p, ui);
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    crate::workspace::chrome::chrome_titlebar(p, ui);
    crate::workspace::chrome::chrome_toolbar(p, ui);
    crate::workspace::chrome::chrome_toolrow(p, ui);
    crate::workspace::view::view_frame(p, ui);
    crate::workspace::chrome::chrome_statusbar(p, ui);
    ui.box_end();
    for f in MOD_FRAME.iter() {
        f(p, ui);
    }
}

/// pn_cmd(id, arg) -> handled
///   ids 100..999: the module of ROUTE_TAB
///   UIC_CLOSE:    fio_cmd only (true keeps the window: "save changes?")
///   UIC_EXIT:     the settings are saved (every end of a real window run, U3)
///   other UIC_*:  broadcast in MOD_CMD order; UIC_THEME / UIC_DPI / UIC_WAKE reach every
///                 module, the others stop at the first module that returns true
pub(super) fn pn_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    // A browser save owns the current document state until storage acknowledges it.
    // Keep wake/timer/render events running, but defer editing, closing and drops.
    if crate::integrations::file_io::fio_save_busy(p)
        && (id < 0x7FFF_0000 || id == UIC_CLOSE || id == UIC_DROP)
    {
        return true;
    }
    if crate::diagnostics::crash::crash_mode(&p.crash) {
        return crate::diagnostics::crash::crash_cmd(p, ui, id, arg);
    }
    if id & 0xFFFF_0000 == 0x7FFF_0000 {
        if id == UIC_EXIT {
            // (U3) the window loop ends in any way (also --auto-close): save the settings
            crate::preferences::set_save_quit(p, ui);
            return true;
        }
        if id == UIC_CLOSE {
            crate::tools::tool_commit_live(p, ui); // T1: a pending tool edit is part of the image
            crate::preferences::set_save_quit(p, ui); // M7: closing the window saves the settings
            return crate::integrations::file_io::fio_cmd(p, ui, id, arg);
        }
        let mut any = false;
        for f in MOD_CMD.iter() {
            let r = f(p, ui, id, arg);
            any |= r;
            if r && id != UIC_THEME && id != UIC_DPI && id != UIC_WAKE {
                break;
            }
        }
        return any;
    }
    // T1: a pending tool edit is committed (or cancelled by Strg+Z) first
    if crate::tools::tool_precmd(p, ui, id, arg) {
        return true;
    }
    // M7: Beenden saves the settings first
    if id == CMD_EXIT {
        crate::preferences::set_save_quit(p, ui);
    }
    for &(lo, hi, f) in ROUTE_TAB.iter() {
        if id >= lo && id <= hi {
            return f(p, ui, id, arg);
        }
    }
    false
}
