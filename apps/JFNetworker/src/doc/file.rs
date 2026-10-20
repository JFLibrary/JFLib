// file -- .jfnet files: reading, writing, the file dialogs, "save changes?", recent files
//
//   read(path) / write(path, &project)      the file as indented JSON (jfn_model's format)
//   request(n, ui, After)                   New / Open / a recent file / Quit: asks first when
//                                           the project has unsaved changes
//   save(n, ui) / save_as(n)                Speichern (Speichern unter without a file)
//   open_path(n, ui, path)                  load a project (--open, recent files, the dialog)
//   open_generated(n, ui, count)            jfn_model::synth's synthetic project, without a file
//   dialogs(n, ui)                          the file dialog and the question, once per frame
//   chosen(n, ui, save) / answer(n, ui, save)   the commands those dialogs queue
//
// The question has three answers: Speichern (then the pending action, after the save dialog
// when the project has no file yet), Verwerfen, Abbrechen.  Errors appear in the info bar above
// the canvas.  The view (zoom, centre) is written into the project's layout when it is saved.
// PERMISSIONS lets the app read the user's chosen files and Dokumente/JFNetworker only, so a
// recent file elsewhere from an earlier session opens the dialog at it (the choice grants it).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::Project;
use ui::*;

use crate::app::constants::*;
use crate::app::Net;

/// the file types of the dialogs
pub(crate) static EXTS: [&str; 1] = ["jfnet=JFNetworker-Projekt"];

/// what follows the question about unsaved changes (or the save it asked for)
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum After {
    New,
    Open,
    OpenPath(String),
    Quit,
}

/// the state of the file dialog, the question and the recent files
pub(crate) struct Files {
    pub fd_open: Shared<i32>,
    /// the dialog saves (else it opens)
    pub fd_save: bool,
    /// the dialog was opened and has not answered yet
    pub fd_active: bool,
    pub fd_path: String,
    pub ask_open: Shared<i32>,
    pub after: Option<After>,
    /// most recent first
    pub recent: Vec<String>,
    /// the info bar: IB_* severity, title, message
    pub msg: Option<(u32, String, String)>,
}

impl Files {
    pub(crate) fn new() -> Files {
        Files {
            fd_open: Shared::new(0),
            fd_save: false,
            fd_active: false,
            fd_path: String::new(),
            ask_open: Shared::new(0),
            after: None,
            recent: Vec::new(),
            msg: None,
        }
    }

    /// path becomes the most recent file
    pub(crate) fn add_recent(&mut self, path: &str) {
        self.recent.retain(|p| !same_path(p, path));
        self.recent.insert(0, String::from(path));
        self.recent.truncate(RECENT_MAX);
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |s: &str| String::from(s).replace('\\', "/");
    if cfg!(windows) { n(a).eq_ignore_ascii_case(&n(b)) } else { n(a) == n(b) }
}

/// the last part of a path
pub(crate) fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(path)
}

/// a .jfnet file -> the project, or why not (German)
pub(crate) fn read(path: &str) -> Result<Project, String> {
    let bytes = plat::read_file(path).ok_or_else(|| String::from("Die Datei ließ sich nicht lesen."))?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let text = core::str::from_utf8(bytes).map_err(|_| String::from("Die Datei ist kein UTF-8-Text."))?;
    Project::from_text(text).map_err(|e| e.text)
}

/// write the project as a .jfnet file (atomically)
pub(crate) fn write(path: &str, p: &Project) -> bool {
    plat::write_file(path, p.to_text().as_bytes())
}

/// where a file dialog starts: the open file, else the last file's folder, else
/// Dokumente/JFNetworker (made when missing); headless runs: the working folder, so scripts
/// can type relative names
fn start_path(n: &Net, ui: &Ui, name: Option<&str>) -> String {
    if let Some(p) = &n.doc.path {
        return p.clone();
    }
    let dir = if ui.headless() {
        plat::cwd()
    } else if let Some(r) = n.file.recent.first() {
        let k = r.rfind(['/', '\\']).unwrap_or(0);
        String::from(&r[..k])
    } else {
        // the app's folder in the documents (PERMISSIONS: always readable and writable)
        match plat::user_dir(plat::DIR_DOCUMENTS) {
            Some(d) => {
                let dir = format!("{}/JFNetworker", d.trim_end_matches(['/', '\\']));
                if plat::file_exists(&dir) == 2 || plat::mkdir(&dir) {
                    dir
                } else {
                    d
                }
            }
            None => plat::cwd(),
        }
    };
    let dir = dir.replace('\\', "/");
    match name {
        Some(f) => format!("{}/{}", dir.trim_end_matches('/'), f),
        None => dir,
    }
}

/// run the action now, or ask first about unsaved changes
pub(crate) fn request(n: &mut Net, ui: &mut Ui, after: After) {
    if n.doc.dirty() {
        n.file.after = Some(after);
        n.file.ask_open.set(1);
        ui.repass();
    } else {
        run(n, ui, after);
    }
}

fn run(n: &mut Net, ui: &mut Ui, after: After) {
    match after {
        After::New => {
            n.doc.replace(Project::new("", 1), None);
            n.after_load();
            n.file.msg = None;
        }
        After::Open => {
            n.file.fd_save = false;
            n.file.fd_path = start_path(n, ui, None);
            n.file.fd_active = true;
            n.file.fd_open.set(1);
        }
        After::OpenPath(p) => {
            if plat::read_file(&p).is_some() {
                open_path(n, ui, &p);
            } else {
                // gone, or outside what the app may read without the user's choice (a
                // file of an earlier session): the dialog at that file grants it
                n.file.fd_save = false;
                n.file.fd_path = p;
                n.file.fd_active = true;
                n.file.fd_open.set(1);
            }
        }
        After::Quit => ui.quit(),
    }
    ui.repass();
}

/// Speichern: into the file, or through the dialog when there is none -> written now
pub(crate) fn save(n: &mut Net, ui: &mut Ui) -> bool {
    match n.doc.path.clone() {
        Some(p) => save_to(n, ui, &p),
        None => {
            save_as(n, ui);
            false
        }
    }
}

/// Speichern unter: the save dialog
pub(crate) fn save_as(n: &mut Net, ui: &mut Ui) {
    n.file.fd_save = true;
    n.file.fd_path = start_path(n, ui, Some("Unbenannt.jfnet"));
    n.file.fd_active = true;
    n.file.fd_open.set(1);
    ui.repass();
}

/// write the project with the current view into path -> it worked
pub(crate) fn save_to(n: &mut Net, ui: &mut Ui, path: &str) -> bool {
    n.canvas.store_view(&mut n.doc.proj);
    if write(path, &n.doc.proj) {
        n.doc.saved(path);
        n.file.add_recent(path);
        n.file.msg = None;
        ui.repass();
        true
    } else {
        n.file.msg = Some((IB_ERROR, format!("„{}“ wurde nicht gespeichert", base_name(path)), String::from("Die Datei ließ sich nicht schreiben.")));
        n.file.after = None;
        ui.repass();
        false
    }
}

/// a path relative to the working folder made absolute ('/' separators; VFS paths stay)
fn absolute(path: &str) -> String {
    let p = path.replace('\\', "/");
    let b = p.as_bytes();
    if p.starts_with('/') || p.contains("://") || (b.len() > 1 && b[1] == b':') {
        return p;
    }
    let cwd = plat::cwd().replace('\\', "/");
    format!("{}/{}", cwd.trim_end_matches('/'), p.trim_start_matches("./"))
}

/// a synthetic project of `count` devices without a file (--gen-topology: performance tests)
pub(crate) fn open_generated(n: &mut Net, ui: &mut Ui, count: usize) {
    n.doc.replace(jfn_model::synth::topology(count), None);
    n.after_load();
    n.file.msg = None;
    ui.repass();
}

/// load a project -> it worked (else the info bar says why)
pub(crate) fn open_path(n: &mut Net, ui: &mut Ui, path: &str) -> bool {
    let path = absolute(path);
    match read(&path) {
        Ok(p) => {
            n.doc.replace(p, Some(path.clone()));
            n.after_load();
            n.file.add_recent(&path);
            n.file.msg = None;
            ui.repass();
            true
        }
        Err(e) => {
            n.file.msg = Some((IB_ERROR, format!("„{}“ ließ sich nicht öffnen", base_name(&path)), e));
            ui.repass();
            false
        }
    }
}

/// the file dialog chose fd_path (CMD_FILE_DONE)
pub(crate) fn chosen(n: &mut Net, ui: &mut Ui, save: bool) {
    let path = n.file.fd_path.clone();
    if save {
        if save_to(n, ui, &path) {
            if let Some(a) = n.file.after.take() {
                run(n, ui, a);
            }
        }
    } else {
        open_path(n, ui, &path);
    }
}

/// the question was answered: Speichern (true) or Verwerfen
pub(crate) fn answer(n: &mut Net, ui: &mut Ui, keep: bool) {
    if keep {
        // without a file the save dialog asks first; `after` waits for it
        if !save(n, ui) {
            return;
        }
    }
    if let Some(a) = n.file.after.take() {
        run(n, ui, a);
    }
}

/// the file dialog and the question (at the end of the frame)
pub(crate) fn dialogs(n: &mut Net, ui: &mut Ui) {
    let open = n.file.fd_open.clone();
    let flags = if n.file.fd_save { FD_SAVE } else { FD_OPEN };
    let ev = ui.filedlg(ID_FILEDLG, flags, &open, &mut n.file.fd_path, 1040, &EXTS);
    if ev & EV_ACT != 0 {
        n.file.fd_active = false;
        ui.cmd_run(CMD_FILE_DONE, n.file.fd_save as u32);
    } else if n.file.fd_active && open.get() == 0 {
        // cancelled: nothing waits for it any more
        n.file.fd_active = false;
        n.file.after = None;
    }
    let ask = n.file.ask_open.clone();
    if ui.dialog_begin(ID_ASK, "Änderungen speichern?", &ask) {
        let text = format!("„{}“ hat ungespeicherte Änderungen. Sollen sie gespeichert werden?", n.doc.file_name());
        ui.next_size(380, 0);
        ui.text(TS_BODY | TX_WRAP, &text);
        match ui.dialog_buttons(Some("Speichern"), Some("Verwerfen"), Some("Abbrechen")) {
            DLG_PRIMARY => ui.cmd_run(CMD_ASK_ANSWER, 1),
            DLG_SECONDARY => ui.cmd_run(CMD_ASK_ANSWER, 0),
            DLG_CLOSE => n.file.after = None,
            _ => {}
        }
    }
    ui.dialog_end();
}
