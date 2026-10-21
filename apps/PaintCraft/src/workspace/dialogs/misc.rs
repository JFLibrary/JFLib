// dlg_misc -- PaintCraft: the Hilfe items.  Info (CMD_HELP_ABOUT) opens the modal window
// "Info": version, Rust/JFLib platform information and copyable diagnostics.
// F1, tutorials and plugin help open the bundled manual. Website and feedback open
// the JFLib project; no project forum or donation endpoint is configured.
//                                              [apps/PaintCraft/src/dlg_misc.s]
//
// Module contract (app/lifecycle.rs):
//   misc_init()                  once at start-up
//   misc_states()                every frame: pn_cmd_set() for CMD_HELP_ABOUT
//   misc_frame()                 after the main layout: the Info window
//   misc_cmd(id, arg) -> bool    true = handled; ids 720-749 (CMD_HELP_DOCS..CMD_WND_LAST)
// Widgets ID_DLG_MISC + n, window WINID_DLG0 + 0x58.

use alloc::format;
use alloc::string::String;

use ui::*;

use crate::app::constants::*;
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const WINID_INFO: u32 = WINID_DLG0 + 0x58; // 0x6068
const ID_INFO_COPY: u32 = ID_DLG_MISC + 2;
const ID_INFO_DIAG: u32 = ID_DLG_MISC + 3;
const DIAG_CAP: usize = 4096;
const S_ARCH: &str = if cfg!(target_arch = "wasm32") { "WebAssembly" } else if cfg!(windows) { "Windows, x64" } else { "Linux, x86-64" };

/// the state of dlg_misc.s (its g_* globals)
pub(crate) struct Misc {
    open: Shared<i32>,
    rect: Shared<WinRect>,
    diag: String,
    help: bool,
}

impl Default for Misc {
    fn default() -> Misc {
        Misc { open: Shared::new(0), rect: Shared::new(WinRect::new(0, 0, 520, 0, 0)), diag: String::new(), help:false }
    }
}

/// misc_init() -- nothing to prepare
pub(crate) fn misc_init(_p: &mut Pn, _ui: &mut Ui) {}

/// misc_states() -- Info is always available
pub(crate) fn misc_states(_p: &mut Pn, ui: &mut Ui) {
    pn_cmd_set(ui, CMD_HELP_ABOUT, 0);
    for id in [CMD_HELP_DOCS,CMD_HELP_TUTORIALS,CMD_HELP_PLUGINS,CMD_HELP_WEBSITE,CMD_HELP_FEEDBACK] {pn_cmd_set(ui,id,0);}
}

/// misc_cmd(id, arg) -> 1 handled -- Info opens the window with fresh diagnostics
pub(crate) fn misc_cmd(p: &mut Pn, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if id==CMD_HELP_WEBSITE || id==CMD_HELP_FEEDBACK {
        ui.open_url(if id==CMD_HELP_WEBSITE {"https://github.com/JFLibrary/JFLib"} else {"https://github.com/JFLibrary/JFLib/issues"});
        return true;
    }
    p.misc.help=matches!(id,CMD_HELP_DOCS|CMD_HELP_TUTORIALS|CMD_HELP_PLUGINS);
    if id != CMD_HELP_ABOUT && !p.misc.help {
        return false;
    }
    let t = crate::workspace::dialogs::settings::set_diag_text(p);
    p.misc.diag = if p.misc.help {String::from(include_str!("../../../docs/HELP.txt"))} else {fmt_cap(t, DIAG_CAP)};
    p.misc.open.set(1);
    ui.repass();
    true
}

/// misc_frame() -- the Info window
pub(crate) fn misc_frame(p: &mut Pn, ui: &mut Ui) {
    let m = &mut p.misc;
    let open = m.open.clone();
    let rect = m.rect.clone();
    if ui.window_begin(WINID_INFO, if m.help {"PaintCraft-Hilfe"} else {"Info"}, Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        if m.help {
            ui.next_size(-1,420);
            ui.textbox(ID_INFO_DIAG,None,&mut m.diag,32768|TB_MULTI|TB_WRAP|TB_READONLY);
            ui.dialog_buttons(None,None,Some(crate::preferences::localization::tr("Schließen")));
        } else {
        ui.text(TS_TITLE, crate::preferences::localization::tr("PaintCraft"));
        let v = fmt_cap(format!("Version {} ({})", crate::preferences::PN_VERSION, S_ARCH), 64);
        ui.text(TS_BODY_STRONG, &v);
        ui.text(
            TS_BODY | TX_WRAP,
            "PaintCraft ist eine Bildbearbeitung in Rust auf Basis von JFLib. Ebenen, Auswahlwerkzeuge, Bildformate, Schriften, Effekte und Oberfläche werden von JFLib bereitgestellt. Die Anwendung unterstützt Windows, Linux und WebAssembly.",
        );
        ui.text(
            TS_CAPTION | TX_WRAP | TX_SECONDARY,
            "paint.net ist eine Marke von dotPDN LLC. PaintCraft ist ein unabhängiges Projekt und steht in keiner Verbindung zu dotPDN LLC.",
        );
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Diagnose"));
        ui.next_size(-1, 180);
        ui.textbox(ID_INFO_DIAG, None, &mut m.diag, DIAG_CAP as u32 | TB_MULTI | TB_WRAP | TB_READONLY);
        if ui.button(ID_INFO_COPY, crate::preferences::localization::tr("In die Zwischenablage kopieren")) & EV_ACT != 0 {
            ui.clip_set(CLIP_TEXT, m.diag.as_bytes());
        }
        ui.dialog_buttons(None, None, Some(crate::preferences::localization::tr("Schließen")));
        }
    }
    ui.window_end();
}
