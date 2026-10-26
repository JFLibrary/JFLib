// files -- the Dateien tab of a device window: the device's files as the project keeps them
// (flash: and nvram: of IOS-like devices, the Linux tree of hosts: jfn_model::files), with size
// and time, the text of the chosen one, deleting it, importing a file from the computer and
// exporting one to it
//
//   has_tab(project, dev)          a device with a file store (IOS-like or a host)
//   tab(n, ui, k, dev)             the space line, the grid, the chosen file's text (or what it
//                                  is), Löschen, Importieren …, Exportieren …
//   dialog(n, ui)                  the file dialog of import and export (once per frame)
//   command(n, ui, id)             CMD_FILES_IMPORT_DONE / CMD_FILES_EXPORT_DONE: the dialog's
//                                  answer
//
// Every change is one undo step of the project ("Datei … auf … löschen"); the engine follows
// it (doc/sim.rs: Action::Files, no restart), as it does the other way round when a transfer
// or a command line changes the files (Report::Files).  An import lands in the chosen
// directory (on a host /root without one, on an IOS-like device in flash:); the system image
// and nvram:startup-config are shown but neither deleted nor exported (the image has no bytes,
// the startup configuration is the Konfiguration tab's).
//
// Ids: ID_FILES0 + 0x180 k (window k): + 1 Löschen, + 2 Importieren, + 3 Exportieren, + 4 the
// text, + 0x10 the grid (its rows + 0x11 + i); ID_FILES_DLG the dialog.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::files::{capacity_of, File, FsErr, Store};
use jfn_model::Project;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

/// the file dialog: what it is for
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ask {
    /// import into this directory of the device
    Import { dev: u32, dir: String },
    /// export this file of the device
    Export { dev: u32, path: String },
}

/// the file dialog of the tab (one for every window)
pub(crate) struct FilesUi {
    fd: Shared<i32>,
    fd_path: String,
    fd_active: bool,
    ask: Option<Ask>,
    /// why the last import or export failed (shown in its window)
    pub err: String,
}

impl FilesUi {
    pub(crate) fn new() -> FilesUi {
        FilesUi { fd: Shared::new(0), fd_path: String::new(), fd_active: false, ask: None, err: String::new() }
    }
}

/// one row of the grid
#[derive(Clone, Debug)]
struct Row {
    key: u64,
    path: String,
    size: u64,
    mtime: i64,
    dir: bool,
    /// the image, nvram:startup-config: shown, not changed here
    fixed: bool,
}

/// a key from a path (FNV-1a)
fn key_of(p: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in p.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h
}

/// the device has files
pub(crate) fn has_tab(p: &Project, dev: u32) -> bool {
    p.model_of(dev).is_some_and(|m| capacity_of(m) > 0)
}

/// the device's store as the project has it
fn store_of(n: &Net, dev: u32) -> Option<Store> {
    let d = n.doc.proj.device(dev)?;
    let m = n.doc.proj.model_of(dev)?;
    let mut s = Store::for_model(m);
    s.load(&d.files);
    Some(s)
}

/// every file and made directory, in path order (IOS-like: flash:, then nvram:)
fn rows_of(n: &Net, dev: u32, st: &Store) -> Vec<Row> {
    let mut v = Vec::new();
    if st.ios {
        for (name, s) in st.list("flash:/").unwrap_or_default() {
            let path = format!("flash:/{}", name);
            v.push(Row { key: key_of(&path), path, size: s.size, mtime: s.mtime, dir: false, fixed: s.builtin });
        }
        let start = n.doc.proj.device(dev).map_or(0, |d| d.startup_config.len() as u64);
        if start > 0 {
            let path = String::from("nvram:/startup-config");
            v.push(Row { key: key_of(&path), path, size: start, mtime: 0, dir: false, fixed: true });
        }
        return v;
    }
    let mut todo = alloc::vec![String::from("/")];
    while let Some(d) = todo.pop() {
        for (name, s) in st.list(&d).unwrap_or_default() {
            let path = if d == "/" { format!("/{}", name) } else { format!("{}/{}", d, name) };
            if s.dir {
                todo.push(path.clone());
                if s.builtin {
                    continue;
                }
            }
            v.push(Row { key: key_of(&path), path, size: s.size, mtime: s.mtime, dir: s.dir, fixed: false });
        }
    }
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

/// "529 B", "12 KiB", "40 MiB"
fn size_text(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{} GiB", b >> 30)
    } else if b >= 1 << 20 {
        format!("{} MiB", b >> 20)
    } else if b >= 10 << 10 {
        format!("{} KiB", b >> 10)
    } else {
        format!("{} B", b)
    }
}

/// "2026-01-01 00:05" of a file time (0: the start of the simulator's clock)
fn time_text(t: i64) -> String {
    let c = datetime::Civil::from_unix(if t == 0 { jfn_sim::EPOCH_UNIX_NS / 1_000_000_000 } else { t });
    format!("{:04}-{:02}-{:02} {:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute)
}

fn cols() -> [GridCol<'static>; 3] {
    [GridCol::new(t("Datei"), 260).flags(GC_NOSORT | GC_FILL), GridCol::new(t("Größe"), 90).flags(GC_NOSORT | GC_RIGHT), GridCol::new(t("Geändert"), 130).flags(GC_NOSORT)]
}

/// the Dateien tab of device window k
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let id = ID_FILES0 + 0x180 * k as u32;
    let Some(st) = store_of(n, dev) else { return };
    let rows = rows_of(n, dev, &st);
    let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
    let keys: Vec<u64> = rows.iter().map(|r| r.key).collect();
    let Some(w) = n.devwin.get(k) else { return };
    w.files.retain(&keys);
    let cur = w.files.cur.and_then(|c| rows.iter().find(|r| r.key == c)).cloned();
    let mut act: Option<u32> = None;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(4), 8);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    let count = rows.iter().filter(|r| !r.dir).count();
    let space = if count == 1 { tf("{} von {} frei (1 Datei)", &[&size_text(st.free()), &size_text(st.capacity)]) } else { tf("{} von {} frei ({} Dateien)", &[&size_text(st.free()), &size_text(st.capacity), &count.to_string()]) };
    ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &format!("{} · {}", if st.ios { "flash:" } else { t("Festplatte") }, space));
    ui.next_icon(IC_OPEN_FILE);
    if ui.button(id + 2, t("Importieren …")) & EV_ACT != 0 {
        act = Some(2);
    }
    ui.tooltip(t("Eine Datei des Computers auf das Gerät kopieren (in das gewählte Verzeichnis)."));
    let can = cur.as_ref().is_some_and(|r| !r.dir && !r.fixed);
    ui.next_flags(if can { 0 } else { ST_DISABLED });
    ui.next_icon(IC_SAVE_AS);
    if ui.button(id + 3, t("Exportieren …")) & EV_ACT != 0 {
        act = Some(3);
    }
    let can_del = cur.as_ref().is_some_and(|r| !r.fixed);
    ui.next_flags(if can_del { 0 } else { ST_DISABLED });
    ui.next_icon(IC_DELETE);
    if ui.button(id + 1, t("Löschen")) & EV_ACT != 0 {
        act = Some(1);
    }
    ui.box_end();
    let err = n.devwin.files.err.clone();
    if !err.is_empty() {
        crate::workspace::error_text(ui, &err, 0);
    }
    let Some(w) = n.devwin.get(k) else { return };
    if rows.is_empty() {
        ui.next_size(-1, 120);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        ui.empty_state(IC_INFO, t("Keine Dateien"), &tf("Auf {} liegt noch nichts. Dateien kommen per TFTP, FTP, über die Befehlszeile oder mit Importieren.", &[&name]));
        ui.box_end();
    } else {
        ui.next_size(-1, 200);
        let (first, end) = ui.grid_begin(id + 0x10, &mut w.files, &cols(), &keys);
        for i in first..end {
            let r = &rows[i as usize];
            ui.grid_row(&mut w.files, i);
            ui.grid_cell(&w.files, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.icon(if r.dir { IC_FOLDER } else { IC_DOCUMENT }, 14);
            ui.text(TS_BODY | TX_ELLIPSIS, &r.path);
            ui.box_end();
            ui.grid_text(&w.files, 1, &if r.dir { String::from("–") } else { size_text(r.size) });
            ui.grid_text(&w.files, 2, &if r.fixed && r.mtime == 0 { String::from("–") } else { time_text(r.mtime) });
        }
        ui.grid_end(&mut w.files);
    }
    // the chosen one: its text, or what it is
    if let Some(r) = &cur {
        let d = n.doc.proj.device(dev);
        let what = if r.path == "nvram:/startup-config" {
            Some(d.map(|d| d.startup_config.clone()).unwrap_or_default())
        } else if r.dir {
            None
        } else {
            st.read(&r.path).ok().filter(|b| jfn_model::files::is_text(b)).map(|b| String::from_utf8_lossy(b).into_owned())
        };
        match what {
            Some(mut text) => {
                ui.next_size(-1, -1);
                ui.textbox(id + 4, None, &mut text, 1_000_000 | TB_MULTI | TB_READONLY);
            }
            None => {
                let note = if r.dir {
                    String::from(t("Ein Verzeichnis."))
                } else if r.fixed {
                    tf("Das Systemabbild des Geräts ({}); es belegt Platz, hat im Simulator aber keinen Inhalt.", &[&size_text(r.size)])
                } else {
                    tf("Binärdaten, {}.", &[&size_text(r.size)])
                };
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &note);
            }
        }
    }
    ui.box_end();
    match act {
        Some(1) => {
            if let Some(r) = cur {
                remove(n, dev, &r.path);
                ui.repass();
            }
        }
        Some(2) => {
            let dir = match &cur {
                Some(r) if r.dir => r.path.clone(),
                Some(r) if !st.ios && !r.fixed => st.parent(&r.path),
                _ => String::from(if st.ios { "flash:/" } else { "/root" }),
            };
            open_dialog(n, Ask::Import { dev, dir });
        }
        Some(3) => {
            if let Some(r) = cur {
                open_dialog(n, Ask::Export { dev, path: r.path });
            }
        }
        _ => {}
    }
}

// one step: the files without `path` (a directory with what is in it)
fn remove(n: &mut Net, dev: u32, path: &str) {
    let Some(name) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return };
    let pre = format!("{}/", path);
    let label = format!("Datei {} auf {} löschen", path, name);
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.files.retain(|f| f.path != path && !f.path.starts_with(&pre))) {
        n.doc.apply(e);
    }
    n.devwin.files.err.clear();
}

fn open_dialog(n: &mut Net, ask: Ask) {
    let f = &mut n.devwin.files;
    let base = match &ask {
        Ask::Export { path, .. } => String::from(path.rsplit(['/', ':']).next().unwrap_or(path)),
        Ask::Import { .. } => String::new(),
    };
    f.fd_path = format!("{}/{}", plat::cwd().replace('\\', "/").trim_end_matches('/'), base);
    f.ask = Some(ask);
    f.fd_active = true;
    f.err.clear();
    f.fd.set(1);
}

/// the file dialog (once per frame)
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let f = &mut n.devwin.files;
    let Some(ask) = f.ask.clone() else { return };
    let open = f.fd.clone();
    let (mode, cmd) = match ask {
        Ask::Import { .. } => (FD_OPEN, CMD_FILES_IMPORT_DONE),
        Ask::Export { .. } => (FD_SAVE, CMD_FILES_EXPORT_DONE),
    };
    let ev = ui.filedlg(ID_FILES_DLG, mode, &open, &mut f.fd_path, 1040, &[]);
    if ev & EV_ACT != 0 {
        f.fd_active = false;
        ui.cmd_run(cmd, 0);
    } else if f.fd_active && open.get() == 0 {
        f.fd_active = false;
        f.ask = None;
    }
}

/// the dialog's answer: the file comes in (one step) or goes out
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    let Some(ask) = n.devwin.files.ask.take() else { return true };
    let path = n.devwin.files.fd_path.clone();
    let file = crate::doc::file::base_name(&path).to_string();
    match (id, ask) {
        (CMD_FILES_EXPORT_DONE, Ask::Export { dev, path: src }) => {
            let data = store_of(n, dev).and_then(|s| s.read(&src).ok().map(Vec::from));
            let ok = data.is_some_and(|d| plat::write_file(&path, &d));
            n.devwin.files.err = if ok { String::new() } else { tf("„{}“ wurde nicht geschrieben.", &[&file]) };
        }
        (CMD_FILES_IMPORT_DONE, Ask::Import { dev, dir }) => match plat::read_file(&path) {
            Some(data) => {
                let Some(st) = store_of(n, dev) else { return true };
                let target = if dir.ends_with('/') { format!("{}{}", dir, file) } else { format!("{}/{}", dir, file) };
                let mut probe = st.clone();
                let t0 = n.doc.sim.engine().clock(dev).map_or(0, |c| c.div_euclid(1_000_000_000));
                match probe.write(&target, &data, t0) {
                    Ok(()) => {
                        let records: Vec<File> = probe.records();
                        let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
                        let label = format!("Datei {} auf {} importieren", target, name);
                        if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.files = records) {
                            n.doc.apply(e);
                        }
                        n.devwin.files.err.clear();
                    }
                    Err(e) => {
                        let why = match e {
                            FsErr::NoSpace => t("kein Platz mehr"),
                            FsErr::NotFound | FsErr::NotDir => t("das Verzeichnis fehlt"),
                            FsErr::IsDir => t("dort liegt ein Verzeichnis"),
                            FsErr::BadName => t("der Name geht nicht"),
                            _ => t("das Gerät nimmt sie nicht"),
                        };
                        n.devwin.files.err = tf("„{}“ kommt nicht auf das Gerät: {}.", &[&file, why]);
                    }
                }
            }
            None => n.devwin.files.err = tf("„{}“ ließ sich nicht lesen.", &[&file]),
        },
        _ => {}
    }
    ui.repass();
    true
}
