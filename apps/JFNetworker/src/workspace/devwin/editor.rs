// editor -- the Texteditor of a host's desktop (desktop.rs): the device's notes and its files
//
//   Editor::new()                    the notes, nothing changed
//   program(n, ui, k, id, dev)       the window's content: Öffnen …, Speichern, Speichern unter
//                                    …, Verwerfen, the file's name and state, the text; the
//                                    chooser in its place while it is open
//   open_file(n, k, dev, path)       a file of the device (None: the notes) into the editor of
//                                    device window k, its window to the front
//   join(dir, name)                  a path in a directory (the chooser, the Dateimanager)
//
// Without a choice the editor shows the device's notes as notizen.txt: Device::comment, saved
// with the project as before (one undo step "Notizen von … speichern"), so older projects keep
// their notes where they were.  Öffnen … and Speichern unter … go through a small chooser over
// the device's files (jfn_model::files, the project's records): a folder's folders and files,
// Nach oben, the notes as a row of their own at the top, a name for Speichern unter; a double
// click enters a folder or takes a file.  Every save is one undo step ("Datei … auf …
// speichern") that the simulator takes at once (Action::Files: the shell's cat sees it).  A
// file the project changes meanwhile (the shell, a transfer, undo) comes into the editor while
// the text there is unchanged.  A file that is no text is not opened.
//
// Ids (id = the desktop block + 0x100): + 0 the text, + 1 Speichern, + 2 Verwerfen, + 3 Öffnen
// …, + 4 Speichern unter …; the chooser + 0x10 Nach oben, + 0x11 the name, + 0x12 Öffnen /
// Speichern, + 0x13 Abbrechen, + 0x80 its list (rows + 0x81 + i).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::{FsErr, Store};
use ui::*;

use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

/// the notes' name in the editor
pub(crate) const NOTES: &str = "notizen.txt";

/// the editor's state in a device's desktop
pub(crate) struct Editor {
    /// the file shown: None the device's notes
    pub path: Option<String>,
    pub text: String,
    /// the text as the project had it (None: not read yet; Some("") also for a missing file)
    src: Option<String>,
    /// the chooser is open: Some(true) Speichern unter, Some(false) Öffnen
    choose: Option<bool>,
    dir: String,
    name: String,
    grid: GridState,
    pub err: String,
}

impl Editor {
    pub(crate) fn new() -> Editor {
        Editor { path: None, text: String::new(), src: None, choose: None, dir: String::from("/root"), name: String::new(), grid: GridState::default(), err: String::new() }
    }
}

/// the device's files as the project keeps them
pub(crate) fn store_of(n: &Net, dev: u32) -> Option<Store> {
    let d = n.doc.proj.device(dev)?;
    let m = n.doc.proj.model_of(dev)?;
    let mut s = Store::for_model(m);
    s.load(&d.files);
    Some(s)
}

/// a path in a directory (a name with "/" or one that starts with it as it is)
pub(crate) fn join(dir: &str, name: &str) -> String {
    if name.starts_with('/') {
        String::from(name)
    } else if dir == "/" || dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

/// the device's clock in seconds (the time of a saved file)
pub(crate) fn now_secs(n: &Net, dev: u32) -> i64 {
    n.doc.sim.engine().clock(dev).map_or(0, |c| c.div_euclid(1_000_000_000))
}

/// why the store refused a file, in words
pub(crate) fn why(e: FsErr) -> &'static str {
    match e {
        FsErr::NoSpace => t("kein Platz mehr"),
        FsErr::NotFound | FsErr::NotDir => t("das Verzeichnis fehlt"),
        FsErr::IsDir => t("dort liegt ein Verzeichnis"),
        FsErr::BadName => t("der Name geht nicht"),
        FsErr::Exists => t("der Name ist vergeben"),
        FsErr::ReadOnly => t("das Gerät erlaubt es nicht"),
        _ => t("das Gerät nimmt sie nicht"),
    }
}

// the text of the file shown as the project has it (None: gone, or no text)
fn current(n: &Net, dev: u32, path: &Option<String>) -> Option<String> {
    match path {
        None => n.doc.proj.device(dev).map(|d| d.comment.clone()),
        Some(p) => {
            let st = store_of(n, dev)?;
            st.read(p).ok().filter(|b| jfn_model::files::is_text(b)).map(|b| String::from_utf8_lossy(b).into_owned())
        }
    }
}

/// a file (None: the notes) into the editor of device window k; its window comes forward
pub(crate) fn open_file(n: &mut Net, k: usize, dev: u32, path: Option<&str>) {
    let text = current(n, dev, &path.map(String::from));
    let Some(w) = n.devwin.get(k) else { return };
    let e = &mut w.desk.edit;
    e.err.clear();
    e.choose = None;
    match text {
        Some(x) => {
            e.path = path.map(String::from);
            e.text = x.clone();
            e.src = Some(x);
        }
        None => e.err = tf("„{}“ ist keine Textdatei.", &[path.unwrap_or(NOTES)]),
    }
    if let Some(p) = &e.path {
        let st_dir = match p.rfind('/') {
            Some(0) | None => String::from("/"),
            Some(i) => String::from(&p[..i]),
        };
        e.dir = st_dir;
    }
    w.desk.open_prog(super::desktop::Prog::Editor);
}

/// the Texteditor of device window k
pub(crate) fn program(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let path = match n.devwin.get(k) {
        Some(w) => w.desk.edit.path.clone(),
        None => return,
    };
    let now = current(n, dev, &path);
    if n.devwin.get(k).is_some_and(|w| w.desk.edit.choose.is_some()) {
        chooser(n, ui, k, id, dev);
        return;
    }
    let Some(w) = n.devwin.get(k) else { return };
    let e = &mut w.desk.edit;
    // the project's text: taken while the editor's text is unchanged
    if e.src != now {
        let unchanged = e.src.is_none() || e.src.as_deref() == Some(e.text.as_str());
        if unchanged && let Some(x) = &now {
            e.text = x.clone();
        }
        e.src = now.clone();
    }
    let changed = e.src.as_deref() != Some(e.text.as_str());
    let mut act = 0u32;
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 6);
    ui.next_icon(IC_OPEN_FILE);
    ui.next_flags(ST_SUBTLE);
    if ui.button(id + 3, t("Öffnen …")) & EV_ACT != 0 {
        act = 3;
    }
    ui.next_flags(if changed { ST_ACCENT } else { ST_DISABLED });
    ui.next_icon(IC_SAVE);
    if ui.button(id + 1, t("Speichern")) & EV_ACT != 0 {
        act = 1;
    }
    ui.next_icon(IC_SAVE_AS);
    ui.next_flags(ST_SUBTLE);
    if ui.button(id + 4, t("Speichern unter …")) & EV_ACT != 0 {
        act = 4;
    }
    ui.next_flags(if changed { ST_SUBTLE } else { ST_SUBTLE | ST_DISABLED });
    if ui.button(id + 2, t("Verwerfen")) & EV_ACT != 0 {
        act = 2;
    }
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.icon(IC_DOCUMENT, 16);
    ui.next_size(-1, 0);
    let shown = e.path.clone().unwrap_or_else(|| String::from(NOTES));
    ui.text(TS_BODY | TX_ELLIPSIS, &shown);
    ui.text(TS_CAPTION | TX_SECONDARY, t(if changed { "geändert" } else { "gesichert" }));
    ui.box_end();
    if !e.err.is_empty() {
        let er = e.err.clone();
        crate::workspace::error_text(ui, &er, 0);
    }
    let Some(w) = n.devwin.get(k) else { return };
    let e = &mut w.desk.edit;
    let notes = e.path.is_none();
    ui.next_size(-1, -1);
    let hint = if notes { Some(t("Notizen zu diesem Gerät: Aufgaben, Zugangsdaten des Labors, was noch fehlt …")) } else { None };
    ui.textbox(id, hint, &mut e.text, if notes { 64_000 } else { 1_000_000 } | TB_MULTI | TB_WRAP);
    match act {
        1 => {
            let (p, text) = (e.path.clone(), e.text.clone());
            save(n, k, dev, p.as_deref(), &text);
            ui.repass();
        }
        2 => {
            e.text = e.src.clone().unwrap_or_default();
            e.err.clear();
            ui.repass();
        }
        3 | 4 => {
            e.choose = Some(act == 4);
            e.name = e.path.as_deref().map(|p| String::from(p.rsplit('/').next().unwrap_or(p))).unwrap_or_else(|| String::from(NOTES));
            e.grid.select_only(None);
            e.err.clear();
            ui.focus(if act == 4 { id + 0x11 } else { id + 0x80 });
            ui.repass();
        }
        _ => {}
    }
}

// one step: the notes (Device::comment) or a file of the device with this text
fn save(n: &mut Net, k: usize, dev: u32, path: Option<&str>, text: &str) -> bool {
    let Some(name) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return false };
    let r = match path {
        None => {
            let text = String::from(text);
            if let Some(e) = edits::device_edit(&n.doc.proj, dev, &format!("Notizen von {} speichern", name), |x| x.comment = text) {
                n.doc.apply(e);
            }
            Ok(String::from(NOTES))
        }
        Some(p) => {
            let t0 = now_secs(n, dev);
            match store_of(n, dev) {
                None => Err(String::from(t("das Gerät nimmt sie nicht"))),
                Some(mut st) => match st.norm(p).and_then(|abs| st.write(&abs, text.as_bytes(), t0).map(|_| abs)) {
                    Ok(abs) => {
                        let records = st.records();
                        if let Some(e) = edits::device_edit(&n.doc.proj, dev, &format!("Datei {} auf {} speichern", abs, name), |x| x.files = records) {
                            n.doc.apply(e);
                        }
                        Ok(abs)
                    }
                    Err(e) => Err(String::from(why(e))),
                },
            }
        }
    };
    let Some(w) = n.devwin.get(k) else { return false };
    let e = &mut w.desk.edit;
    match r {
        Ok(abs) => {
            e.path = path.map(|_| abs);
            e.text = String::from(text);
            e.src = Some(String::from(text));
            e.err.clear();
            true
        }
        Err(why) => {
            e.err = tf("„{}“ wurde nicht gespeichert: {}.", &[path.unwrap_or(NOTES), &why]);
            false
        }
    }
}

/// a row of the chooser
struct Row {
    key: u64,
    /// None: the notes
    path: Option<String>,
    name: String,
    dir: bool,
}

fn key_of(p: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in p.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h | 2
}

// the chooser of Öffnen … and Speichern unter …
fn chooser(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let st = store_of(n, dev);
    let Some(w) = n.devwin.get(k) else { return };
    let e = &mut w.desk.edit;
    let save_as = e.choose == Some(true);
    if st.as_ref().is_some_and(|s| !s.is_dir(&e.dir)) {
        e.dir = String::from("/root");
    }
    let mut rows: Vec<Row> = alloc::vec![Row { key: 1, path: None, name: format!("{} ({})", NOTES, t("Notizen des Geräts")), dir: false }];
    if let Some(s) = &st {
        let list = s.list(&e.dir).unwrap_or_default();
        for (name, sta) in list.iter().filter(|x| x.1.dir).chain(list.iter().filter(|x| !x.1.dir)) {
            let p = join(&e.dir, name);
            rows.push(Row { key: key_of(&p), path: Some(p), name: name.clone(), dir: sta.dir });
        }
    }
    let keys: Vec<u64> = rows.iter().map(|r| r.key).collect();
    e.grid.retain(&keys);
    let mut act = 0u32;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY_STRONG, t(if save_as { "Speichern unter" } else { "Datei öffnen" }));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &e.dir);
    ui.next_icon(IC_UP);
    ui.next_flags(if e.dir == "/" { ST_SUBTLE | ST_DISABLED } else { ST_SUBTLE });
    if ui.button(id + 0x10, t("Nach oben")) & EV_ACT != 0 {
        act = 0x10;
    }
    ui.box_end();
    let cols = [GridCol::new(t("Name"), 240).flags(GC_NOSORT | GC_FILL)];
    ui.next_size(-1, -1);
    let (first, end) = ui.grid_begin(id + 0x80, &mut e.grid, &cols, &keys);
    for i in first..end {
        let r = &rows[i as usize];
        ui.grid_row(&mut e.grid, i);
        ui.grid_cell(&e.grid, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.icon(if r.path.is_none() { IC_EDIT } else if r.dir { IC_FOLDER } else { IC_DOCUMENT }, 14);
        ui.text(TS_BODY | TX_ELLIPSIS, &r.name);
        ui.box_end();
    }
    let gev = ui.grid_end(&mut e.grid);
    let cur = e.grid.cur.and_then(|c| rows.iter().position(|r| r.key == c));
    // a click on a file gives its name to Speichern unter
    if gev.ev & EV_ACT != 0
        && save_as
        && let Some(r) = cur.map(|i| &rows[i])
        && !r.dir
    {
        e.name = r.path.as_deref().map(|p| String::from(p.rsplit('/').next().unwrap_or(p))).unwrap_or_else(|| String::from(NOTES));
    }
    if gev.ev & EV_DOUBLE != 0 && gev.row >= 0 {
        act = 0x100 + gev.row as u32;
    }
    if save_as {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY | TX_SECONDARY, t("Name"));
        ui.next_size(-1, 0);
        let ev = ui.textbox(id + 0x11, None, &mut e.name, 255);
        if ev & EV_COMMIT != 0 && ui.focused() == id + 0x11 {
            act = 0x12;
        }
        ui.box_end();
    }
    if !e.err.is_empty() {
        let er = e.err.clone();
        crate::workspace::error_text(ui, &er, 0);
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    let Some(w) = n.devwin.get(k) else { return };
    let e = &mut w.desk.edit;
    if ui.button(id + 0x13, t("Abbrechen")) & EV_ACT != 0 {
        act = 0x13;
    }
    let ready = if save_as { !e.name.trim().is_empty() } else { cur.is_some() };
    ui.next_flags(if ready { 0 } else { ST_DISABLED });
    if ui.accent_button(id + 0x12, t(if save_as { "Speichern" } else { "Öffnen" })) & EV_ACT != 0 {
        act = 0x12;
    }
    ui.box_end();
    // what the buttons and the list asked for
    let pick = |i: usize| (rows[i].path.clone(), rows[i].dir);
    match act {
        0x10 => {
            e.dir = match e.dir.rfind('/') {
                Some(0) | None => String::from("/"),
                Some(i) => String::from(&e.dir[..i]),
            };
            e.grid.select_only(None);
            ui.repass();
        }
        0x13 => {
            e.choose = None;
            e.err.clear();
            ui.repass();
        }
        0x12 | 0x100.. => {
            let chosen = if act >= 0x100 { Some((act - 0x100) as usize) } else { cur };
            match chosen.map(pick) {
                // a folder: into it
                Some((Some(p), true)) => {
                    e.dir = p;
                    e.grid.select_only(None);
                }
                _ if save_as && act == 0x12 => {
                    let name = String::from(e.name.trim());
                    let text = e.text.clone();
                    // the notes' row chosen with their name: the notes, else a file in the folder
                    let target = if cur == Some(0) && name == NOTES { None } else { Some(join(&e.dir, &name)) };
                    if save(n, k, dev, target.as_deref(), &text)
                        && let Some(w) = n.devwin.get(k)
                    {
                        w.desk.edit.choose = None;
                    }
                }
                // Speichern unter: a double click on a file takes its name
                Some((Some(p), false)) if save_as => e.name = String::from(p.rsplit('/').next().unwrap_or(&p)),
                Some((None, _)) if save_as => e.name = String::from(NOTES),
                Some((p, false)) => open_file(n, k, dev, p.as_deref()),
                _ => {}
            }
            ui.repass();
        }
        _ => {}
    }
}
