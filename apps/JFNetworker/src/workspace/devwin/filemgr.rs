// filemgr -- the Dateimanager of a host's desktop (desktop.rs): the device's folders and files
//
//   Fm::new()                         /root shown, nothing chosen
//   program(n, ui, k, id, dev)        the window's content: the tool row (Neuer Ordner,
//                                     Umbenennen, Löschen, Kopieren, Ausschneiden, Einfügen,
//                                     Öffnen, Eigenschaften, Importieren …, Exportieren …), the
//                                     folder's path with Nach oben, the folders as a tree beside
//                                     the folder's entries (Name, Größe, Art, Geändert), a line
//                                     for a new name, the properties of the chosen entry
//   paste_name(st, dir, name)         a free name in a folder: name, "name (Kopie)", "name
//                                     (Kopie 2)" ... (before a file's extension)
//   copy_into(st, from, dir, t) / move_into(st, from, dir)   a file or a folder with all in it
//                                     into a folder of the store (Einfügen) -> the new path
//
// Every change is one undo step of the project ("Ordner … auf … anlegen", "… umbenennen", "…
// löschen", "… einfügen"), made as the Dateien tab makes them (edits::device_edit with the
// records of the project's store, jfn_model::files); the simulator follows at once (Action::
// Files).  Kopieren and Ausschneiden keep paths (per desktop, for the session); Einfügen copies
// them into the folder shown (a folder with all in it; a taken name gets " (Kopie)") or moves
// them there (the times stay).  A double click opens a folder, or a file with its program
// (desktop::open_with: scripts through open_jfs, the rest in the Texteditor).  Importieren … and
// Exportieren … are the Dateien tab's file dialogs (files.rs): into the folder shown, the chosen
// file out -- this is where a .jfs script comes in and goes out.  The system's folders (/etc,
// /root ...) cannot be renamed or deleted.  While the list has the keyboard Entf deletes, F2
// renames, Strg+C / X / V copy, cut and paste, Eingabe opens.
//
// Ids (id = the desktop block + 0x200): + 1 .. + 10 the tool row, + 0x10 Nach oben, + 0x11 the
// new name, + 0x12 OK, + 0x13 Abbrechen, + 0x14 the properties' Schließen; + 0x40 the tree
// (rows + 0x41 + i), + 0x100 the list (rows + 0x101 + i).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::files::{FsErr, Store};
use ui::*;

use super::editor::{join, now_secs, store_of, why};
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

/// what the line below the tool row asks for
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ask {
    NewFolder,
    Rename(String),
}

/// the Dateimanager's state in a device's desktop
pub(crate) struct Fm {
    /// the folder shown
    pub dir: String,
    tree: TreeState,
    pub grid: GridState,
    /// Kopieren / Ausschneiden: the paths and whether they move
    clip: Option<(Vec<String>, bool)>,
    ask: Option<Ask>,
    name: String,
    /// the properties of this entry are shown
    props: Option<String>,
    pub err: String,
}

impl Fm {
    pub(crate) fn new() -> Fm {
        let mut tree = TreeState::default();
        tree.expanded.insert(key_of("/"));
        Fm { dir: String::from("/root"), tree, grid: GridState::default(), clip: None, ask: None, name: String::new(), props: None, err: String::new() }
    }
}

fn key_of(p: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in p.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h
}

fn parent(p: &str) -> String {
    match p.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(i) => String::from(&p[..i]),
    }
}

fn base_name(p: &str) -> &str {
    if p == "/" { "/" } else { p.rsplit('/').next().unwrap_or(p) }
}

/// a free name for `name` in folder `dir`: as it is, else " (Kopie)", " (Kopie 2)" ... before
/// the extension of a file ("a.txt" -> "a (Kopie).txt")
pub(crate) fn paste_name(st: &Store, dir: &str, name: &str) -> String {
    if !st.exists(&join(dir, name)) {
        return String::from(name);
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for k in 1.. {
        let cand = if k == 1 { format!("{} (Kopie){}", stem, ext) } else { format!("{} (Kopie {}){}", stem, k, ext) };
        if !st.exists(&join(dir, &cand)) {
            return cand;
        }
    }
    String::from(name)
}

/// a copy of a file or of a folder with all in it into folder `dir` (a free name) -> its path
pub(crate) fn copy_into(st: &mut Store, from: &str, dir: &str, t0: i64) -> Result<String, FsErr> {
    let s = st.stat(from).ok_or(FsErr::NotFound)?;
    let to = join(dir, &paste_name(st, dir, base_name(from)));
    if st.within(&to, from) {
        return Err(FsErr::BadName);
    }
    if !s.dir {
        let data = Vec::from(st.read(from)?);
        st.write(&to, &data, t0)?;
        return Ok(to);
    }
    st.mkdir(&to, t0, false)?;
    let mut todo = alloc::vec![(String::from(from), to.clone())];
    while let Some((src, dst)) = todo.pop() {
        for (name, sub) in st.list(&src)? {
            let (a, b) = (join(&src, &name), join(&dst, &name));
            if sub.dir {
                st.mkdir(&b, t0, false)?;
                todo.push((a, b));
            } else {
                let data = Vec::from(st.read(&a)?);
                st.write(&b, &data, t0)?;
            }
        }
    }
    Ok(to)
}

/// a file or folder moved into folder `dir` (a free name there; the times stay) -> its path
pub(crate) fn move_into(st: &mut Store, from: &str, dir: &str) -> Result<String, FsErr> {
    if parent(from) == dir {
        return Ok(String::from(from));
    }
    let to = join(dir, &paste_name(st, dir, base_name(from)));
    st.rename(from, &to)?;
    Ok(to)
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

/// an entry of the folder shown
#[derive(Clone, Debug)]
struct Entry {
    key: u64,
    path: String,
    name: String,
    dir: bool,
    size: u64,
    mtime: i64,
    builtin: bool,
    kind: &'static str,
}

// what a file is (i18n)
fn kind_of(st: &Store, path: &str, dir: bool) -> &'static str {
    if dir {
        return "Ordner";
    }
    if path.ends_with(".jfs") {
        return "Skript (jfs)";
    }
    match st.read(path) {
        Ok(b) if jfn_model::files::is_text(b) => "Textdatei",
        _ => "Datei",
    }
}

fn entries(st: &Store, dir: &str) -> Vec<Entry> {
    let list = st.list(dir).unwrap_or_default();
    list.iter()
        .filter(|x| x.1.dir)
        .chain(list.iter().filter(|x| !x.1.dir))
        .map(|(name, s)| {
            let path = join(dir, name);
            Entry { key: key_of(&path), kind: kind_of(st, &path, s.dir), path, name: name.clone(), dir: s.dir, size: s.size, mtime: s.mtime, builtin: s.builtin }
        })
        .collect()
}

// the folders as tree rows (depth first, the open ones' children): key, depth, children, path
fn folders(st: &Store, tree: &TreeState) -> Vec<(u64, u16, bool, String)> {
    let mut out = Vec::new();
    let mut todo = alloc::vec![(String::from("/"), 0u16)];
    while let Some((p, d)) = todo.pop() {
        let subs: Vec<String> = st.list(&p).unwrap_or_default().into_iter().filter(|x| x.1.dir).map(|x| join(&p, &x.0)).collect();
        let k = key_of(&p);
        out.push((k, d, !subs.is_empty(), p));
        if tree.expanded.contains(&k) {
            for s in subs.into_iter().rev() {
                todo.push((s, d + 1));
            }
        }
    }
    out
}

/// one step of the project: the device's files as `st` has them
fn commit(n: &mut Net, dev: u32, st: &Store, label: &str) {
    let records = st.records();
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, label, |x| x.files = records) {
        n.doc.apply(e);
    }
}

// the tool row's buttons: (part, icon, label)
// i18n
const TOOLS: [(u32, u32, &str); 10] = [
    (1, IC_FOLDER, "Neuer Ordner"),
    (2, IC_EDIT, "Umbenennen"),
    (3, IC_DELETE, "Löschen"),
    (4, IC_COPY, "Kopieren"),
    (5, IC_CUT, "Ausschneiden"),
    (6, IC_PASTE, "Einfügen"),
    (7, IC_OPEN_FILE, "Öffnen"),
    (8, IC_INFO, "Eigenschaften"),
    (9, IC_DOWNLOAD, "Importieren …"),
    (10, IC_UPLOAD, "Exportieren …"),
];

/// the Dateimanager of device window k
pub(crate) fn program(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(st) = store_of(n, dev) else { return };
    // why the last import or export failed (the Dateien tab's dialog)
    let dlg_err = n.devwin.files.err.clone();
    let Some(w) = n.devwin.get(k) else { return };
    let fm = &mut w.desk.fm;
    if !st.is_dir(&fm.dir) {
        fm.dir = String::from("/root");
    }
    let list = entries(&st, &fm.dir);
    let keys: Vec<u64> = list.iter().map(|e| e.key).collect();
    fm.grid.retain(&keys);
    let chosen: Vec<Entry> = fm.grid.selected_in_order().iter().filter_map(|k| list.iter().find(|e| e.key == *k)).cloned().collect();
    let one = (chosen.len() == 1).then(|| chosen[0].clone());
    let changeable = !chosen.is_empty() && chosen.iter().all(|e| !e.builtin);
    let mut act = 0u32;
    // the tool row (icons with their names for screen readers and as tooltips)
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 2);
    for (part, icon, label) in TOOLS {
        let on = match part {
            2 => one.as_ref().is_some_and(|e| !e.builtin),
            3 | 5 => changeable,
            4 => !chosen.is_empty(),
            6 => fm.clip.is_some(),
            7 | 8 => one.is_some(),
            10 => one.as_ref().is_some_and(|e| !e.dir),
            _ => true,
        };
        ui.next_icon(icon);
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE | if on { 0 } else { ST_DISABLED });
        if ui.button(id + part, t(label)) & EV_ACT != 0 {
            act = part;
        }
        if part == 3 || part == 6 || part == 8 {
            ui.separator();
        }
    }
    ui.box_end();
    // the path
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_UP);
    ui.next_flags(ST_ICON_ONLY | ST_SUBTLE | if fm.dir == "/" { ST_DISABLED } else { 0 });
    if ui.button(id + 0x10, t("Nach oben")) & EV_ACT != 0 {
        act = 0x10;
    }
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_ELLIPSIS, &fm.dir);
    ui.box_end();
    // a new name
    if let Some(ask) = fm.ask.clone() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY | TX_SECONDARY, t(if ask == Ask::NewFolder { "Neuer Ordner" } else { "Neuer Name" }));
        ui.next_size(-1, 0);
        let ev = ui.textbox(id + 0x11, None, &mut fm.name, 255);
        if ev & EV_COMMIT != 0 && ui.focused() == id + 0x11 {
            act = 0x12;
        }
        if ui.focused() == id + 0x11 && ui.capture_shortcut(VK_ESCAPE, 0) {
            act = 0x13;
        }
        ui.next_flags(if fm.name.trim().is_empty() { ST_DISABLED } else { 0 });
        if ui.accent_button(id + 0x12, t("OK")) & EV_ACT != 0 {
            act = 0x12;
        }
        if ui.button(id + 0x13, t("Abbrechen")) & EV_ACT != 0 {
            act = 0x13;
        }
        ui.box_end();
    }
    for e in [fm.err.clone(), dlg_err] {
        if !e.is_empty() {
            crate::workspace::error_text(ui, &e, 0);
        }
    }
    // the folders beside the folder's entries
    let rows_data = folders(&st, &fm.tree);
    let rows: Vec<TreeRow> = rows_data.iter().map(|r| TreeRow { key: r.0, depth: r.1, has_children: r.2, check: None, icon: IC_FOLDER, label: base_name(&r.3), detail: None }).collect();
    fm.tree.cur = Some(key_of(&fm.dir));
    let props = fm.props.clone().and_then(|p| list.iter().find(|e| e.path == p).cloned());
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(170, -1);
    let tev = ui.tree(id + 0x40, &mut fm.tree, &rows, 0);
    if tev.sel_changed
        && let Some(r) = fm.tree.cur.and_then(|c| rows_data.iter().find(|r| r.0 == c))
    {
        fm.dir = r.3.clone();
        fm.grid.select_only(None);
        fm.props = None;
        ui.repass();
    }
    if tev.expanded.is_some() || tev.collapsed.is_some() {
        ui.repass();
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 6);
    if list.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        ui.empty_state(IC_FOLDER, t("Der Ordner ist leer"), t("Neuer Ordner legt einen an, Einfügen und Importieren … bringen Dateien hierher."));
        ui.box_end();
    } else {
        let cols = [
            GridCol::new(t("Name"), 200).flags(GC_NOSORT | GC_FILL),
            GridCol::new(t("Größe"), 74).flags(GC_NOSORT | GC_RIGHT),
            GridCol::new(t("Art"), 90).flags(GC_NOSORT),
            GridCol::new(t("Geändert"), 124).flags(GC_NOSORT),
        ];
        ui.next_size(-1, -1);
        let (first, end) = ui.grid_begin(id + 0x100, &mut fm.grid, &cols, &keys);
        for i in first..end {
            let e = &list[i as usize];
            ui.grid_row(&mut fm.grid, i);
            ui.grid_cell(&fm.grid, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.icon(if e.dir { IC_FOLDER } else { IC_DOCUMENT }, 14);
            ui.text(TS_BODY | TX_ELLIPSIS, &e.name);
            ui.box_end();
            ui.grid_text(&fm.grid, 1, &if e.dir { String::from("–") } else { size_text(e.size) });
            ui.grid_text(&fm.grid, 2, t(e.kind));
            ui.grid_text(&fm.grid, 3, &if e.builtin { String::from("–") } else { time_text(e.mtime) });
        }
        let gev = ui.grid_end(&mut fm.grid);
        if gev.ev & EV_DOUBLE != 0 && gev.row >= 0 {
            act = 0x100 + gev.row as u32;
        }
        // the keys of the list
        if ui.focused() == id + 0x100 && fm.ask.is_none() {
            if ui.key(VK_DELETE) && changeable {
                act = 3;
            } else if ui.key(VK_F2) && one.as_ref().is_some_and(|e| !e.builtin) {
                act = 2;
            } else if ui.capture_shortcut(b'C' as u32, MOD_CTRL) && !chosen.is_empty() {
                act = 4;
            } else if ui.capture_shortcut(b'X' as u32, MOD_CTRL) && changeable {
                act = 5;
            } else if ui.capture_shortcut(b'V' as u32, MOD_CTRL) && fm.clip.is_some() {
                act = 6;
            }
        }
    }
    // the properties, or what is chosen
    if let Some(e) = &props {
        ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 2);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY_STRONG | TX_ELLIPSIS, &e.name);
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        if ui.button(id + 0x14, t("Schließen")) & EV_ACT != 0 {
            act = 0x14;
        }
        ui.box_end();
        let size = if e.dir { tf("{} in Dateien darin", &[&size_text(st.under(&e.path))]) } else { tf("{} ({} Bytes)", &[&size_text(e.size), &e.size.to_string()]) };
        let rows = [(t("Art"), String::from(t(e.kind))), (t("Ort"), parent(&e.path)), (t("Größe"), size), (t("Geändert"), if e.builtin { String::from(t("vom System")) } else { time_text(e.mtime) })];
        for (k, v) in rows {
            ui.box_begin(0, BOX_H, 8);
            ui.next_size(80, 0);
            ui.text(TS_CAPTION | TX_SECONDARY, k);
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_ELLIPSIS, &v);
            ui.box_end();
        }
        ui.box_end();
    } else {
        ui.next_size(-1, 0);
        let line = match (chosen.len(), &fm.clip) {
            (0, None) => tf("Einträge: {}", &[&list.len().to_string()]),
            (0, Some((c, cut))) => tf(if *cut { "Einträge: {}; zum Verschieben bereit: {}" } else { "Einträge: {}; zum Kopieren bereit: {}" }, &[&list.len().to_string(), &c.len().to_string()]),
            (1, _) => tf("{} gewählt", &[&chosen[0].name]),
            (c, _) => tf("{} von {} gewählt", &[&c.to_string(), &list.len().to_string()]),
        };
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &line);
    }
    ui.box_end();
    ui.box_end();
    if act == 0 {
        return;
    }
    ui.repass();
    run(n, ui, k, id, dev, st, act, &list, &chosen);
}

// what a button, a key or a double click asked for
#[allow(clippy::too_many_arguments)]
fn run(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32, mut st: Store, act: u32, list: &[Entry], chosen: &[Entry]) {
    let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
    let t0 = now_secs(n, dev);
    let Some(w) = n.devwin.get(k) else { return };
    let fm = &mut w.desk.fm;
    let dir = fm.dir.clone();
    fm.err.clear();
    match act {
        1 => {
            fm.ask = Some(Ask::NewFolder);
            fm.name = paste_name(&st, &dir, t("Neuer Ordner"));
            ui.focus(id + 0x11);
        }
        2 => {
            if let Some(e) = chosen.first() {
                fm.ask = Some(Ask::Rename(e.path.clone()));
                fm.name = e.name.clone();
                ui.focus(id + 0x11);
            }
        }
        3 => {
            let mut fail = None;
            for e in chosen {
                let r = if e.dir { st.remove_tree(&e.path) } else { st.remove(&e.path) };
                if let Err(x) = r {
                    fail = Some(tf("„{}“ bleibt: {}.", &[&e.name, why(x)]));
                    break;
                }
            }
            match fail {
                Some(f) => fm.err = f,
                None => {
                    fm.grid.select_only(None);
                    fm.props = None;
                    let label = if chosen.len() == 1 { format!("{} auf {} löschen", chosen[0].path, name) } else { format!("{} Einträge auf {} löschen", chosen.len(), name) };                    commit(n, dev, &st, &label);
                }
            }
        }
        4 | 5 => fm.clip = Some((chosen.iter().map(|e| e.path.clone()).collect(), act == 5)),
        6 => {
            let Some((paths, cut)) = fm.clip.clone() else { return };
            let mut done: Vec<String> = Vec::new();
            let mut fail = None;
            for p in &paths {
                let r = if cut { move_into(&mut st, p, &dir) } else { copy_into(&mut st, p, &dir, t0) };
                match r {
                    Ok(x) => done.push(x),
                    Err(x) => {
                        fail = Some(tf("„{}“ kommt nicht hierher: {}.", &[base_name(p), why(x)]));
                        break;
                    }
                }
            }
            match fail {
                Some(f) => fm.err = f,
                None => {
                    if cut {
                        fm.clip = None;
                    }
                    let keys: Vec<u64> = done.iter().map(|p| key_of(p)).collect();
                    fm.grid.sel = keys.iter().copied().collect();
                    fm.grid.cur = keys.last().copied();
                    let label = format!("{} Einträge auf {} in {} einfügen", done.len(), name, dir);
                    commit(n, dev, &st, &label);
                }
            }
        }
        7 => {
            if let Some(e) = chosen.first().cloned() {
                open_entry(n, k, dev, &e);
            }
        }
        8 => fm.props = chosen.first().map(|e| e.path.clone()),
        9 => crate::workspace::devwin::files::import_into(n, dev, &dir),
        10 => {
            if let Some(e) = chosen.first() {
                crate::workspace::devwin::files::export_from(n, dev, &e.path);
            }
        }
        0x10 => {
            fm.dir = parent(&dir);
            fm.grid.select_only(None);
            fm.props = None;
        }
        0x12 => {
            let new = String::from(fm.name.trim());
            let ask = fm.ask.take();
            let r = match &ask {
                Some(Ask::NewFolder) => st.mkdir(&join(&dir, &new), t0, false).map(|_| (join(&dir, &new), format!("Ordner {} auf {} anlegen", join(&dir, &new), name))),
                Some(Ask::Rename(p)) => {
                    let to = join(&parent(p), &new);
                    st.rename(p, &to).map(|_| (to.clone(), format!("{} auf {} in {} umbenennen", p, name, new)))
                }
                None => return,
            };
            match r {
                Ok((path, label)) => {
                    fm.grid.select_only(Some(key_of(&path)));
                    commit(n, dev, &st, &label);
                }
                Err(x) => {
                    fm.err = tf("„{}“ geht nicht: {}.", &[&new, why(x)]);
                    fm.ask = ask;
                }
            }
        }
        0x13 => fm.ask = None,
        0x14 => fm.props = None,
        a if a >= 0x100 => {
            if let Some(e) = list.get((a - 0x100) as usize).cloned() {
                open_entry(n, k, dev, &e);
            }
        }
        _ => {}
    }
}

// a folder opens in the Dateimanager, a file in its program
fn open_entry(n: &mut Net, k: usize, dev: u32, e: &Entry) {
    if e.dir {
        if let Some(w) = n.devwin.get(k) {
            let fm = &mut w.desk.fm;
            fm.tree.expanded.insert(key_of(&parent(&e.path)));
            fm.dir = e.path.clone();
            fm.grid.select_only(None);
            fm.props = None;
        }
        return;
    }
    super::desktop::open_with(n, k, dev, &e.path);
}
