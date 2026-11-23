// ask_save -- (JD15) changes are never lost without asking (UI-12, KB-01), and the ways a file
// opens (DM-09, UI-06, UI-05)
//
//   Next                          what waits for the answer: Close (Datei › Schließen, Strg+W,
//                                 Strg+F4), Quit (the window's close), a template's form (Neu ›
//                                 Erstellen), a file (Öffnen, with its Mode)
//   Mode                          Edit; ReadOnly (Speichern goes to Speichern unter, the title says
//                                 so); Copy (a new, unnamed drawing from the file: Öffnen › Als
//                                 Kopie öffnen, Neu › Neu aus vorhandener Zeichnung)
//   command(app, ui, id)          App::command first: Schließen and UIC_CLOSE (true keeps the
//                                 window while the question is shown)
//   guard(app, ui, next) -> bool  true: go on now (the drawing has no changes); false: the question
//                                 "Möchten Sie die Änderungen an „Name“ speichern?" is shown and
//                                 `next` waits for it
//   open(app, ui, path, mode)     a file from Öffnen (its dialog, Zuletzt verwendet): stencils open
//                                 beside the drawing without asking; a file that cannot be read
//                                 shows its notice at once (DF-15: the File register closes)
//   dialog(app, ui)               the question, every pass: Speichern (Speichern unter's dialog for
//                                 a new or read-only drawing) and then the action, Nicht speichern
//                                 and the action, Abbrechen leaves everything as it was
//   saved(app, ui)                edit_doc::save_to after writing: the action that waited goes on
//   read_only(app)                the drawing was opened read-only
//
// The changes are the undo history's (History::dirty, mark_saved in save_to); the text being
// edited goes into the drawing before the question.  A save dialog closed without saving drops
// the action.  An action that goes on after the answer passes the guard (going_on), so
// templates::create and open run as they would without changes.

use alloc::string::String;
use ui::i18n::{t, tf};
use ui::*;

use crate::Diagrammer;

const ID_DLG: u32 = 0x7900_0000;

/// what waits for the answer
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Next {
    Close,
    Quit,
    /// Neu › Erstellen: the catalogue's entry and form
    Template(usize, usize),
    /// Öffnen: the file and how it opens
    Open(String, Mode),
}

/// how a file opens
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum Mode {
    #[default]
    Edit,
    ReadOnly,
    Copy,
}

/// the question's state
pub(crate) struct AskSave {
    pub open: Shared<i32>,
    pending: Option<Next>,
    /// Speichern was the answer: the action after the drawing is written
    after_save: Option<Next>,
    going_on: bool,
    /// how the file Öffnen's dialog gives opens (Durchsuchen, Als Kopie öffnen, Neu aus
    /// vorhandener Zeichnung)
    pub dlg_mode: Mode,
    /// Datei › Öffnen › Schreibgeschützt öffnen (the check box)
    pub read_only: i32,
}

impl Default for AskSave {
    fn default() -> AskSave {
        AskSave { open: Shared::new(0), pending: None, after_save: None, going_on: false, dlg_mode: Mode::Edit, read_only: 0 }
    }
}

/// whether the open drawing has changes that are not saved
pub(crate) fn dirty(app: &Diagrammer) -> bool {
    app.opened.as_ref().is_some_and(|o| o.ed.history.dirty())
}

/// whether the open drawing was opened read-only (DM-09)
pub(crate) fn read_only(app: &Diagrammer) -> bool {
    app.opened.as_ref().is_some_and(|o| o.ed.read_only)
}

/// App::command before the others: Schließen and the window's close -> handled
pub(crate) fn command(app: &mut Diagrammer, ui: &mut Ui, id: u32) -> bool {
    match id {
        // true keeps the window: the question is open (or opens now)
        UIC_CLOSE => app.ask.open.get() != 0 || !guard(app, ui, Next::Quit),
        crate::commands::CLOSE => {
            if guard(app, ui, Next::Close) {
                crate::viewer::close(app, ui);
            }
            ui.repass();
            true
        }
        _ => false,
    }
}

/// true: go on with `next` now; false: the question is shown, `next` waits for its answer
pub(crate) fn guard(app: &mut Diagrammer, ui: &mut Ui, next: Next) -> bool {
    if app.ask.going_on {
        return true;
    }
    // the text being edited is part of the drawing
    if app.text.is_some() {
        crate::edit_text::finish(app, ui);
    }
    if !dirty(app) {
        return true;
    }
    app.ask.pending = Some(next);
    app.ask.after_save = None;
    app.ask.open.set(1);
    ui.repass();
    false
}

/// open a file from Datei › Öffnen (its dialog, Zuletzt verwendet) -> opened
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui, path: &str, mode: Mode) -> bool {
    let l = path.to_ascii_lowercase();
    // a stencil goes into the Shapes panel, the drawing stays (JD17: also .vsx and .vss)
    let stencil = l.ends_with(".vssx") || l.ends_with(".vssm") || l.ends_with(".vsx") || l.ends_with(".vss");
    if !stencil && !guard(app, ui, Next::Open(String::from(path), mode)) {
        return false;
    }
    if !crate::viewer::open(app, ui, path) {
        // (DF-15) the notice at once: from the File register back to the drawing
        if app.error.is_some() {
            app.backstage = None;
        }
        return false;
    }
    // the file itself is the open drawing (not a stencil, not a new drawing from a template)
    let same = app.opened.as_ref().and_then(|o| o.ed.path.as_deref()) == Some(path);
    if let (true, Some(o)) = (same, app.opened.as_mut()) {
        match mode {
            Mode::Edit => {}
            Mode::ReadOnly => o.ed.read_only = true,
            Mode::Copy => {
                o.ed.path = None;
                o.name = crate::edit_doc::next_name();
            }
        }
    }
    ui.repass();
    true
}

/// the question (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    // Speichern unter's dialog closed without saving: the action does not happen
    if app.ask.after_save.is_some() && app.save_dlg.get() == 0 {
        app.ask.after_save = None;
    }
    let open = app.ask.open.clone();
    if !ui.dialog_begin(ID_DLG, t("Änderungen speichern"), &open) {
        return;
    }
    let name = app.drawing_name();
    ui.next_size(380, 0);
    ui.text(TS_BODY | TX_WRAP, &tf("Möchten Sie die Änderungen an „{}“ speichern?", &[&name]));
    let r = ui.dialog_buttons(Some(t("Speichern")), Some(t("Nicht speichern")), Some(t("Abbrechen")));
    ui.dialog_end();
    match r {
        DLG_PRIMARY => {
            app.ask.after_save = app.ask.pending.take();
            crate::edit_doc::save(app, ui, false);
        }
        DLG_SECONDARY => {
            if let Some(next) = app.ask.pending.take() {
                proceed(app, ui, next);
            }
        }
        DLG_CLOSE => app.ask.pending = None,
        _ => {}
    }
    if r != DLG_NONE {
        ui.repass();
    }
}

/// the drawing was written (edit_doc::save_to): the action that waited for it goes on
pub(crate) fn saved(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(next) = app.ask.after_save.take() {
        proceed(app, ui, next);
    }
}

// the action after the answer (the guard lets it pass)
fn proceed(app: &mut Diagrammer, ui: &mut Ui, next: Next) {
    app.ask.going_on = true;
    match next {
        Next::Close => crate::viewer::close(app, ui),
        Next::Quit => ui.quit(),
        Next::Template(k, f) => {
            crate::templates::create(app, ui, k, f);
        }
        Next::Open(path, mode) => {
            open(app, ui, &path, mode);
        }
    }
    app.ask.going_on = false;
    ui.repass();
}
