// fileio -- Neu / Öffnen / Speichern / Speichern unter, the "save changes?" prompt, the
// window title and the error bar                              [apps/paint/src/fileio.s]
//
//   file_frame(p, ui)              dialogs (call at the end of the frame)
//   file_infobar(p, ui)            error bar (call where it should appear)
//   file_cmd(p, ui, id, arg) -> bool  handled; UIC_CLOSE: true = keep open
//   file_title(p, ui)              window title from the document path
//   open_file(p, ui, path) -> 0 / -1
//   paint_error(p, msg)            show msg in the error bar
//   doc_name(d) -> file name of the document ("Unbenannt" when there is none)

use alloc::format;
use alloc::string::String;

use ui::*;

use crate::consts::*;
use crate::dialogs::dialogs_close;
use crate::doc::{doc_composite, doc_layer, doc_mark_all, doc_modified, doc_new, doc_set_modified, Doc};
use crate::history::hist_clear;
use crate::view::view_fit;
use crate::{fmt_cap, Kritzel};

const ID_FILEDLG: u32 = 0x0010_0000; // .. + 0x3FFF (file dialog)
const ID_ASKDLG: u32 = 1700;
const ID_FILEBAR: u32 = 1710;
const PATH_CAP: u32 = 1040;

/// the extension list of the file dialog (order = CMD_SAVE_PNG..GIF)
static EXTS: [&str; 4] = ["png=PNG-Bild", "jpg;jpeg;jpe;jfif=JPEG-Bild", "bmp;dib=BMP-Bild", "gif=GIF-Bild"];

/// the file state (the .data / .bss of fileio.s)
pub(crate) struct FileIo {
    fd_mode: u32,
    /// the error bar's message (g_err_msg; None = no bar)
    pub(crate) err_msg: Option<&'static str>,
    fd_open: Shared<i32>,
    ask_open: Shared<i32>,
    pending: u32,
    after_save: u32,
    fd_path: String,
}

impl Default for FileIo {
    fn default() -> FileIo {
        FileIo {
            fd_mode: FD_OPEN,
            err_msg: None,
            fd_open: Shared::new(0),
            ask_open: Shared::new(0),
            pending: 0,
            after_save: 0,
            fd_path: String::new(),
        }
    }
}

/// file_frame() -- the file dialog and the "save changes?" dialog
pub(crate) fn file_frame(p: &mut Kritzel, ui: &mut Ui) {
    let f = &mut p.file;
    if ui.filedlg(ID_FILEDLG, f.fd_mode, &f.fd_open, &mut f.fd_path, PATH_CAP, &EXTS) & EV_ACT != 0 {
        ui.cmd_run(CMD_FILE_DONE, f.fd_mode);
    }
    let ask = f.ask_open.clone();
    if ui.dialog_begin(ID_ASKDLG, "Kritzel", &ask) {
        let msg = fmt_cap(format!("Die Änderungen an „{}“ sind noch nicht gesichert. Jetzt speichern?", doc_name(&p.doc)), 512);
        ui.text(TS_BODY | TX_WRAP, &msg);
        let r = ui.dialog_buttons(Some("Speichern"), Some("Nicht speichern"), Some("Abbrechen"));
        if r != 0 {
            ui.cmd_run(CMD_ASK_DONE, r);
        }
    }
    ui.dialog_end();
}

/// paint_error(msg) -- show msg in the error bar (static string)
pub(crate) fn paint_error(p: &mut Kritzel, msg: &'static str) {
    p.file.err_msg = Some(msg);
}

/// file_infobar() -- error bar after a failed open / save
pub(crate) fn file_infobar(p: &mut Kritzel, ui: &mut Ui) {
    let Some(m) = p.file.err_msg else { return };
    if ui.infobar(ID_FILEBAR, IB_ERROR, "Fehler", m) & EV_ACT != 0 {
        p.file.err_msg = None;
    }
}

/// file_cmd(id, arg) -> true handled (UIC_CLOSE: true = keep the window), false otherwise
pub(crate) fn file_cmd(p: &mut Kritzel, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        UIC_CLOSE => {
            // the window's close button: ask first when there are changes
            if !doc_modified(&p.doc) {
                return false; // close
            }
            request(p, ui, CMD_EXIT);
        }
        CMD_EXIT | CMD_NEW | CMD_OPEN => request(p, ui, id),
        CMD_SAVE => {
            if p.doc.path.is_empty() {
                show_dialog(p, FD_SAVE); // PNG preselected
            } else {
                let path = p.doc.path.clone();
                save_to(p, ui, &path);
            }
        }
        CMD_SAVE_AS => show_dialog(p, FD_SAVE),
        CMD_SAVE_PNG..=CMD_SAVE_GIF => {
            // a file type from the "Speichern unter" submenu: preselected in the dialog
            show_dialog(p, (id - CMD_SAVE_TYPE0) << FD_EXT_SHIFT | FD_SAVE);
        }
        CMD_FILE_DONE => {
            // the file dialog confirmed fd_path (arg = its mode)
            let path = p.file.fd_path.clone();
            if arg & FD_SAVE == 0 {
                open_file(p, ui, &path);
            } else if save_to(p, ui, &path) == 0 {
                // saved: continue what the prompt interrupted
                let a = p.file.after_save;
                p.file.after_save = 0;
                if a != 0 {
                    do_action(p, ui, a);
                }
            }
        }
        CMD_ASK_DONE => {
            let a = p.file.pending;
            p.file.pending = 0;
            if arg == DLG_SECONDARY {
                do_action(p, ui, a); // "Nicht speichern"
            } else if arg == DLG_PRIMARY {
                if p.doc.path.is_empty() {
                    p.file.after_save = a;
                    show_dialog(p, FD_SAVE);
                } else {
                    let path = p.doc.path.clone();
                    if save_to(p, ui, &path) == 0 {
                        do_action(p, ui, a);
                    }
                }
            } // else "Abbrechen"
        }
        _ => return false,
    }
    true
}

// request(action) -- CMD_NEW / CMD_OPEN / CMD_EXIT, asking about unsaved changes first
fn request(p: &mut Kritzel, ui: &mut Ui, action: u32) {
    if !doc_modified(&p.doc) {
        do_action(p, ui, action);
        return;
    }
    p.file.pending = action;
    p.file.fd_open.set(0);
    dialogs_close(p);
    p.file.ask_open.set(1);
}

// do_action(action) -- carry out CMD_NEW / CMD_OPEN / CMD_EXIT without asking
fn do_action(p: &mut Kritzel, ui: &mut Ui, action: u32) {
    match action {
        CMD_NEW => {
            hist_clear(&mut p.hist);
            doc_new(p, DEF_W, DEF_H);
            view_fit(p, ui);
            file_title(p, ui);
        }
        CMD_OPEN => show_dialog(p, FD_OPEN),
        CMD_EXIT => ui.quit(),
        _ => {}
    }
}

// str_copy(dst, src, cap): at most cap - 1 bytes
fn copy_cap(s: &str, cap: usize) -> String {
    let mut n = s.len().min(cap - 1);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&s[..n])
}

// show_dialog(mode) -- open the file dialog, starting at the document's path
fn show_dialog(p: &mut Kritzel, mode: u32) {
    let f = &mut p.file;
    f.fd_mode = mode;
    f.fd_path = copy_cap(&p.doc.path, PATH_CAP as usize);
    if mode & FD_SAVE != 0 {
        // save: "folder/name" without the extension, the chosen file type adds its own
        if f.fd_path.is_empty() {
            f.fd_path = String::from("Unbenannt");
        } else {
            let base = f.fd_path.rfind('/').unwrap_or(0);
            if let Some(dot) = f.fd_path[base..].rfind('.') {
                f.fd_path.truncate(base + dot);
            }
        }
    }
    f.fd_open.set(1);
}

// save_to(path) -> 0 ok -- save the flattened image, adopt the path
fn save_to(p: &mut Kritzel, ui: &mut Ui, path: &str) -> i32 {
    doc_composite(&mut p.doc);
    let r = img_core::io::save(path, &p.doc.comp.borrow());
    if r != 0 {
        p.file.err_msg = Some("Speichern fehlgeschlagen: Das Bild ließ sich nicht schreiben.");
        return -1;
    }
    if p.doc.path != path {
        p.doc.path = copy_cap(path, PATH_CAP as usize);
    }
    doc_set_modified(&mut p.doc, false);
    file_title(p, ui);
    p.file.err_msg = None;
    0
}

/// open_file(path) -> 0 ok -- load an image as a new one-layer document
pub(crate) fn open_file(p: &mut Kritzel, ui: &mut Ui, path: &str) -> i32 {
    const FAIL: &str = "Öffnen fehlgeschlagen: Die Datei ist kein lesbares Bild.";
    let sf = match img_core::io::load(path) {
        Ok(s) => s,
        Err(_) => {
            p.file.err_msg = Some(FAIL);
            return -1;
        }
    };
    if sf.w as u32 > DOC_MAX as u32 || sf.h as u32 > DOC_MAX as u32 {
        p.file.err_msg = Some(FAIL);
        return -1;
    }
    hist_clear(&mut p.hist);
    if doc_new(p, sf.w, sf.h) != 0 {
        p.file.err_msg = Some(FAIL);
        return -1;
    }
    img_core::ops::blit_copy(doc_layer(&mut p.doc, 0), &sf, 0, 0);
    // transparent pixels need the checkerboard
    p.doc.bg_opaque = !img_core::meta::all_opaque(&sf) as i32;
    p.doc.path = copy_cap(path, PATH_CAP as usize);
    doc_mark_all(&mut p.doc);
    doc_set_modified(&mut p.doc, false);
    view_fit(p, ui);
    file_title(p, ui);
    p.file.err_msg = None;
    0
}

/// doc_name() -> file name of the document ("Unbenannt" when there is none)
pub(crate) fn doc_name(d: &Doc) -> &str {
    let s = d.path.as_str();
    if s.is_empty() {
        return "Unbenannt";
    }
    // "C:/x\y.png": the later separator wins
    match s.rfind(['/', '\\']) {
        Some(i) => &s[i + 1..],
        None => s,
    }
}

/// file_title() -- "name - Kritzel"
pub(crate) fn file_title(p: &mut Kritzel, ui: &mut Ui) {
    let t = fmt_cap(format!("{} - Kritzel", doc_name(&p.doc)), 300);
    ui.set_title(&t);
}
