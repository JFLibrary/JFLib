// shell -- the main window (FEATURES.md 3, UI-01): the title bar with the quick access toolbar,
// the register row and the ribbon, the dock area (the Shapes panel on the left, the canvas in
// the middle, the task panes on the right), the status bar; the File register's backstage in
// place of everything below the title bar
//
//   frame(app, ui)    the whole window, built on every pass
//
// The title bar is the app's own (AF_CUSTOM_TITLEBAR): it carries the quick access toolbar like
// Visio's (UI-30) and follows the theme, so it is dark in the dark theme on every system
// (JD0's note 7: the system frame stayed light).  The register row is ui.ribbon_tabs: File,
// the four registers, the context registers the selection asks for; it also runs the access
// keys (Alt or F10, UI-37); the quick access buttons answer to 1, 2, 3.  Under the ribbon (JD4)
// the notice of an open drawing (read-only, substituted fonts, macros) or of a file that could
// not be opened; the file dialog of Backstage › Öffnen runs here on every pass.
// (JD15) Without a drawing (Datei › Schließen) the window shows no page, no page tabs, no
// Shapes panel and an empty status bar, only what can come next (Neu, Öffnen); the title bar
// names a drawing opened read-only so (DM-09).

use alloc::format;
use ui::*;

use crate::{backstage, panes, ribbon, Diagrammer};

pub(crate) const ID_TABS: u32 = 100;
const ID_NOTICE: u32 = 130;
const ID_OPEN_DLG: u32 = 0x7000;
const ID_SAVE_DLG: u32 = 0x7100;
const ID_EMPTY: u32 = 0x7920_0000;

/// the main window
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    let title = match (&app.opened, crate::ask_save::read_only(app)) {
        // (JD15) no drawing open; a drawing opened read-only (DM-09)
        (None, _) if app.preview == ribbon::SEL_NONE => alloc::string::String::from("JFDiagrammer"),
        (_, true) => format!("{} – JFDiagrammer", ui::i18n::tf("{} [Schreibgeschützt]", &[&app.drawing_name()])),
        _ => format!("{} – JFDiagrammer", app.drawing_name()),
    };
    if ui.title() != title {
        ui.set_title(&title);
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    titlebar(app, ui, &title);
    open_dialog(app, ui);
    save_dialog(app, ui);
    if app.backstage.is_some() {
        backstage::frame(app, ui);
        ui.box_end();
        return;
    }
    // the register row: File, the registers, the context ones (the selection kind decides)
    let regs = ribbon::tabs(app);
    let mut sel = regs.iter().position(|r| *r == app.reg).unwrap_or(1) as i32;
    let descs = ribbon::tab_descs(ui, &regs);
    let ev = ui.ribbon_tabs(ID_TABS, &mut sel, &descs);
    app.reg = regs.get(sel.max(1) as usize).copied().unwrap_or(ribbon::Reg::Start);
    if ev & EV_OPEN != 0 {
        app.open_backstage(ui, app.last_page);
    }
    ribbon::draw(app, ui, app.reg);
    notice(app, ui);
    // (JD15, UI-12) no drawing: nothing of the last one stays (page tabs, status bar)
    if app.opened.is_none() && app.preview == ribbon::SEL_NONE {
        empty(ui);
        ui.box_end();
        return;
    }
    // the dock area: the Shapes panel, the canvas, the task panes
    let panels = panes::panels();
    ui.next_size(-1, -1);
    ui.dock_begin(panes::ID_DOCK, &mut app.panes, &panels);
    while let Some(p) = ui.dock_next(&mut app.panes) {
        if p == DOCK_CENTER {
            panes::center(app, ui);
        } else {
            panes::content(app, ui, p);
        }
    }
    if ui.dock_end(&mut app.panes) {
        app.save_layout();
    }
    panes::status(app, ui);
    ui.box_end();
}

// the notice under the ribbon: a file that could not be opened, else the open drawing's (closable)
fn notice(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some((title, msg)) = app.error.clone() {
        ui.next_size(-1, 0);
        if ui.infobar(ID_NOTICE, 3, &title, &msg) & EV_ACT != 0 {
            app.error = None;
            ui.repass();
        }
        return;
    }
    let Some((title, msg)) = app.opened.as_ref().and_then(|o| o.notice.clone()) else { return };
    ui.next_size(-1, 0);
    if ui.infobar(ID_NOTICE, 0, &title, &msg) & EV_ACT != 0 {
        if let Some(o) = app.opened.as_mut() {
            o.notice = None;
        }
        ui.repass();
    }
}

// the file dialog of Backstage › Öffnen (shown while app.open_dlg is set)
fn open_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.open_dlg.clone();
    let exts: alloc::vec::Vec<&str> = crate::viewer::EXTS.iter().map(|e| ui::i18n::t(e)).collect();
    if ui.filedlg(ID_OPEN_DLG, FD_OPEN, &dlg, &mut app.open_path, 4096, &exts) & EV_ACT != 0 {
        let path = app.open_path.clone();
        // (JD15) as Durchsuchen, Als Kopie öffnen or Neu aus vorhandener Zeichnung asked, after
        // the question about the changes (ask_save.rs)
        let mode = core::mem::take(&mut app.ask.dlg_mode);
        crate::ask_save::open(app, ui, &path, mode);
    }
}

// (JD15) the window without a drawing: what can come next, an empty status bar
fn empty(ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(ID_EMPTY, BOX_V | BOX_CENTER | BOX_XCENTER | BOX_BG, 12);
    ui.text(TS_SUBTITLE, ui::i18n::t("Keine Zeichnung geöffnet"));
    ui.text(TS_BODY | TX_SECONDARY, ui::i18n::t("Erstellen Sie eine neue Zeichnung, oder öffnen Sie eine vorhandene."));
    ui.box_begin(ID_EMPTY + 1, BOX_H, 8);
    // (own widget ids: the quick access toolbar may show the same commands)
    for (k, id) in [crate::commands::NEW, crate::commands::OPEN].into_iter().enumerate() {
        ui.next_icon(crate::commands::def(id).map_or(0, |d| d.icon));
        if ui.button(ID_EMPTY + 2 + k as u32, crate::commands::label(id)) & EV_ACT != 0 {
            ui.cmd_run(id, 0);
        }
    }
    ui.box_end();
    ui.box_end();
    ui.statusbar_begin();
    ui.statusbar_end();
}

// (JD7) the file dialog of Speichern unter (shown while app.save_dlg is set)
fn save_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.save_dlg.clone();
    let exts: alloc::vec::Vec<&str> = crate::viewer::SAVE_EXTS.iter().map(|e| ui::i18n::t(e)).collect();
    if ui.filedlg(ID_SAVE_DLG, FD_SAVE, &dlg, &mut app.save_path, 4096, &exts) & EV_ACT != 0 {
        // the path as the dialog granted it (fs.user)
        let path = ui.filedlg_paths().into_iter().next().unwrap_or_else(|| app.save_path.clone());
        crate::edit_doc::save_to(app, ui, &path);
    }
}

// the title bar: the quick access toolbar (JD8 rest: qat.rs, its commands and menu) and the
// drawing's name
fn titlebar(app: &mut Diagrammer, ui: &mut Ui, title: &str) {
    ui.titlebar_begin_opt(None, 0);
    crate::qat::buttons(app, ui);
    crate::qat::more(app, ui);
    ui.spacer(-1);
    ui.text(TS_BODY | TX_ELLIPSIS, title);
    ui.titlebar_end();
}
