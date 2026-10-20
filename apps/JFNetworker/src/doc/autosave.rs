// autosave -- copies of unsaved work and their recovery after a crash
//
//   Autosave                    this session's copy (folder, id, lock), the timer, the offer
//   frame(n, ui)                every frame: the timer (TIMER_AUTOSAVE, the interval of the
//                               settings; 0 switches it off)
//   tick(n)                     the timer went off: a copy when the project has unsaved changes
//                               the last copy does not have
//   flush(n)                    UIC_SUSPEND / UIC_SESSION_END: a copy now, whatever the interval
//   clean(n)                    the project was saved, replaced or the app ends with nothing
//                               unsaved: this session's copy goes (and an adopted one)
//   discard(n)                  Verwerfen before quitting: the same, and the exit keeps nothing
//   startup(n, ui)              copies of sessions that are not running any more: the newest
//                               one is offered in the recovery bar above the canvas
//   bar(n, ui)                  the recovery bar above the canvas: Wiederherstellen, Verwerfen,
//                               × (later: the copy stays for the next start)
//   recover(n, ui) / drop(n)    its buttons (CMD_RECOVER, CMD_RECOVER_DROP)
//
// A copy is <folder>/<id>.jfnet (the project as Speichern writes it, with the view) and
// <folder>/<id>.ini (the file it belongs to, its name, when, how many changes).  The folder is
// data://autosave; headless runs use mem://jfnetworker/autosave so tests never touch the
// user's folders, or --autosave-dir DIR (tests of the recovery across two runs).  A running
// session holds <id>.lock (a FileLock: across processes), so a second window never offers the
// copy of the first.  A copy that cannot be read is offered nowhere and removed with the others
// it belongs to by Verwerfen.  A recovered project keeps the file it came from and counts as
// unsaved; the session adopts the recovered copy (holds its lock) and removes it only once its
// own copy is written or nothing is left to keep, so a second crash loses nothing.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::Project;
use plat::FileLock;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;

/// a copy of another session the bar offers
#[derive(Clone, Debug)]
pub(crate) struct Offer {
    pub id: String,
    /// the file it belongs to (None: never saved)
    pub path: Option<String>,
    /// its name as the title showed it ("Büro.jfnet", "Unbenannt")
    pub name: String,
    /// when it was written (µs of the task clock), the newest is offered
    pub at: u64,
}

pub(crate) struct Autosave {
    /// the folder of the copies
    pub dir: String,
    /// this session's id (hex) and its lock (taken with the first copy)
    pub id: String,
    lock: Option<FileLock>,
    /// the Doc::rev of the last copy (0: none)
    saved_rev: u32,
    /// the timer runs with this interval (ms; 0: not set)
    armed_ms: u32,
    /// what the recovery bar offers
    pub offer: Option<Offer>,
    /// copies written by this session (scripts, the selftest)
    pub writes: u32,
    /// the recovered copy of another session and its lock, held until this session keeps its
    /// own copy or nothing is left to keep
    adopted: Option<(String, Option<FileLock>)>,
    /// Verwerfen before quitting: the exit keeps no copy
    discarded: bool,
}

impl Autosave {
    pub(crate) fn new() -> Autosave {
        Autosave {
            dir: String::new(),
            id: String::new(),
            lock: None,
            saved_rev: 0,
            armed_ms: 0,
            offer: None,
            writes: 0,
            adopted: None,
            discarded: false,
        }
    }

    fn file(&self, id: &str, ext: &str) -> String {
        format!("{}/{}.{}", self.dir, id, ext)
    }

    /// the project at this revision has its copy (the inspector says so)
    pub(crate) fn has_copy(&self, rev: u32) -> bool {
        self.saved_rev != 0 && self.saved_rev == rev
    }

    /// Wiederherstellen took the copy `id`: this session holds it (no other window offers it)
    pub(crate) fn adopt(&mut self, id: &str) {
        self.release();
        let lock = FileLock::acquire(&self.file(id, "lock"));
        self.adopted = Some((String::from(id), lock));
    }

    /// the adopted copy goes: this session's own copy replaces it, or the work was saved,
    /// replaced or dropped
    pub(crate) fn release(&mut self) {
        if let Some((id, lock)) = self.adopted.take() {
            drop(lock);
            remove(&self.dir, &id);
        }
    }
}

// the folder and this session's id (once)
fn prepare(n: &mut Net, ui: &Ui) {
    let a = &mut n.autosave;
    if !a.dir.is_empty() {
        return;
    }
    a.dir = match plat::arg_value("--autosave-dir") {
        Some(d) => String::from(d.trim_end_matches(['/', '\\'])),
        None if ui.headless() => String::from("mem://jfnetworker/autosave"),
        None => String::from("data://autosave"),
    };
    let mut r = [0u8; 8];
    plat::fill_random(&mut r);
    a.id = r.iter().map(|b| format!("{:02x}", b)).collect();
}

/// the timer: set while autosave is on, again when its interval changed
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    prepare(n, ui);
    let ms = n.prefs.autosave_s.saturating_mul(1000);
    if ms != n.autosave.armed_ms {
        n.autosave.armed_ms = ms;
        if ms > 0 {
            ui.timer(TIMER_AUTOSAVE, ms);
        }
    }
}

/// TIMER_AUTOSAVE: a copy when there is something new to keep; the timer goes on
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    if n.prefs.autosave_s == 0 {
        n.autosave.armed_ms = 0;
        return;
    }
    if n.doc.dirty() && n.doc.rev != n.autosave.saved_rev {
        write(n);
    }
    ui.timer(TIMER_AUTOSAVE, n.autosave.armed_ms.max(1000));
}

/// UIC_SUSPEND / UIC_SESSION_END: a copy now (the session may not come back)
pub(crate) fn flush(n: &mut Net) {
    if !n.autosave.dir.is_empty() && n.doc.dirty() && n.doc.rev != n.autosave.saved_rev {
        write(n);
    }
}

// write this session's copy and its description
fn write(n: &mut Net) -> bool {
    let a = &mut n.autosave;
    if a.lock.is_none() {
        plat::mkdir(&a.dir);
        a.lock = FileLock::acquire(&a.file(&a.id.clone(), "lock"));
    }
    let mut p = n.doc.proj.clone();
    n.canvas.store_view(&mut p);
    let a = &mut n.autosave;
    if !plat::write_file(&a.file(&a.id.clone(), "jfnet"), p.to_text().as_bytes()) {
        return false;
    }
    let mut ini = base::Ini::new();
    ini.set("autosave", "path", n.doc.path.as_deref().unwrap_or(""));
    ini.set("autosave", "name", &n.doc.file_name());
    ini.set("autosave", "at", &format!("{}", plat::task::now_us()));
    ini.set("autosave", "rev", &format!("{}", n.doc.rev));
    let a = &mut n.autosave;
    let ok = ini.save(&a.file(&a.id.clone(), "ini"));
    a.saved_rev = n.doc.rev;
    a.writes += 1;
    // the work is in this session's copy now
    a.release();
    ok
}

/// this session's copy goes (and an adopted one): the project was saved or replaced, or the
/// app ends cleanly
pub(crate) fn clean(n: &mut Net) {
    let a = &mut n.autosave;
    a.saved_rev = 0;
    a.release();
    if a.lock.is_none() || a.dir.is_empty() {
        return;
    }
    let id = a.id.clone();
    for ext in ["jfnet", "ini"] {
        plat::delete(&a.file(&id, ext));
    }
}

/// Verwerfen when the app quits: the changes go, so do the copies; the exit keeps none
pub(crate) fn discard(n: &mut Net) {
    n.autosave.discarded = true;
    clean(n);
}

/// UIC_EXIT: with nothing unsaved (or the changes dropped) the copy and the lock go; unsaved
/// changes (the session ends, the system shuts down) keep the copy for the next start
pub(crate) fn exit(n: &mut Net) {
    if n.doc.dirty() && !n.autosave.discarded {
        flush(n);
        return;
    }
    clean(n);
    let a = &mut n.autosave;
    if a.lock.take().is_some() {
        let id = a.id.clone();
        plat::delete(&a.file(&id, "lock"));
    }
}

/// the copies of sessions that are not running (their lock is free), newest first
pub(crate) fn orphans(dir: &str) -> Vec<Offer> {
    let mut ids: Vec<String> = Vec::new();
    plat::dir_list(dir, &mut |name, is_dir, _| {
        if !is_dir {
            if let Some(id) = name.strip_suffix(".jfnet") {
                ids.push(String::from(id));
            }
        }
    });
    let mut out: Vec<Offer> = Vec::new();
    for id in ids {
        // a running session holds its lock
        let Some(probe) = FileLock::acquire(&format!("{}/{}.lock", dir, id)) else { continue };
        drop(probe);
        let ini = base::Ini::load(&format!("{}/{}.ini", dir, id));
        let path = ini.get("autosave", "path").filter(|p| !p.is_empty()).map(String::from);
        let name = ini.get("autosave", "name").map(String::from).unwrap_or_else(|| String::from(t("Unbenannt")));
        let at = ini.get("autosave", "at").and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(0);
        out.push(Offer { id, path, name, at });
    }
    out.sort_by(|a, b| b.at.cmp(&a.at).then(a.id.cmp(&b.id)));
    out
}

/// at the start: offer the newest copy of a session that did not end well
pub(crate) fn startup(n: &mut Net, ui: &mut Ui) {
    prepare(n, ui);
    let dir = n.autosave.dir.clone();
    n.autosave.offer = orphans(&dir).into_iter().next();
    if n.autosave.offer.is_some() {
        ui.repass();
    }
}

/// Wiederherstellen: the copy becomes the open project (unsaved, with the file it belongs to);
/// this session adopts it until its own copy replaces it (a copy that cannot be read goes)
pub(crate) fn recover(n: &mut Net, ui: &mut Ui) {
    let Some(o) = n.autosave.offer.take() else { return };
    let dir = n.autosave.dir.clone();
    let text = plat::read_file(&format!("{}/{}.jfnet", dir, o.id)).and_then(|b| String::from_utf8(b).ok());
    match text.as_deref().map(Project::from_text) {
        Some(Ok(p)) => {
            n.doc.replace(p, o.path.clone());
            n.doc.hist.mark_unsaved();
            n.after_load();
            n.autosave.adopt(&o.id);
            n.file.msg = Some((IB_SUCCESS, tf("„{}“ wiederhergestellt", &[&o.name]), String::from(t("Die automatisch gesicherten Änderungen sind wieder da; speichern übernimmt sie in die Datei."))));
        }
        _ => {
            n.file.msg = Some((IB_ERROR, String::from(t("Die Sicherung ließ sich nicht lesen")), String::from(t("Sie wurde verworfen."))));
            remove(&dir, &o.id);
        }
    }
    ui.repass();
}

/// Verwerfen: the copy goes, the next one (if any) is offered
pub(crate) fn drop_offer(n: &mut Net, ui: &mut Ui) {
    let Some(o) = n.autosave.offer.take() else { return };
    let dir = n.autosave.dir.clone();
    remove(&dir, &o.id);
    n.autosave.offer = orphans(&dir).into_iter().next();
    ui.repass();
}

// the files of a copy
fn remove(dir: &str, id: &str) {
    for ext in ["jfnet", "ini", "lock"] {
        plat::delete(&format!("{}/{}.{}", dir, id, ext));
    }
}

/// the recovery bar above the canvas (while a copy is offered)
pub(crate) fn bar(n: &mut Net, ui: &mut Ui) {
    let Some(o) = n.autosave.offer.as_ref().map(|o| o.name.clone()) else { return };
    let mut later = false;
    ui.box_begin(0, BOX_V | pad(8), 0);
    ui.next_size(-1, 0);
    ui.box_begin(ID_RECOVER_BAR, BOX_H | BOX_XCENTER | BOX_CARD | pad(8), 10);
    ui.icon(IC_WARNING, 20);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 2);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, &tf("Ungespeicherte Änderungen an „{}“ gefunden", &[&o]));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("JFNetworker wurde nicht ordentlich beendet; eine automatische Sicherung ist da."));
    ui.box_end();
    if ui.accent_button(ID_RECOVER_OK, t("Wiederherstellen")) & EV_ACT != 0 {
        ui.cmd_run(CMD_RECOVER, 0);
    }
    if ui.button(ID_RECOVER_DROP, t("Verwerfen")) & EV_ACT != 0 {
        ui.cmd_run(CMD_RECOVER_DROP, 0);
    }
    // later: the copy stays for the next start
    ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
    if ui.icon_button(ID_RECOVER_LATER, IC_CANCEL, t("Später")) & EV_ACT != 0 {
        later = true;
    }
    ui.box_end();
    ui.box_end();
    if later {
        n.autosave.offer = None;
        ui.repass();
    }
}
