// ide -- the steps of the IDE (workspace/ide) on a device's files and script services: saving
// files, a new file or folder, a new name, deleting, the conversion of an inline script into a
// file and a script service bound to a file -- each one undo step of the project
//
//   store_of(proj, dev) -> Option<Store>   the device's files as the project has them
//   text_of(store, path) -> Option<String> a text file's content (None: missing, a folder, no
//                                    UTF-8 text)
//   save(doc, dev, path, text) -> Result<bool, String>   the file gets the text (Ok(false):
//                                    it had it); a new file is made, its folder must be there
//   save_many(doc, dev, files, label) -> Result<usize, String>   several files in one step (Alle
//                                    speichern, replace in files) -> how many changed
//   new_file(doc, dev, path, text) / new_dir(doc, dev, path) -> Result<String, String>   the
//                                    canonical path; refused when it exists or its folder does
//                                    not
//   rename(doc, dev, from, to) -> Result<String, String>   a file or a folder with all in it
//                                    gets another path (the new one)
//   remove(doc, dev, path)           a file, or a folder with all in it
//   new_unit(doc, dev, name, exec) -> Result<String, String>   the unit file NAME.service for
//                                    the script `exec`, its folder made (A2) -> its path
//   file_of(def) -> Option<&str>     the "file" of a script service's definition
//   bound(d, store, path) -> Vec<String>   the script services of the device that run the file
//   bind(doc, dev, path) -> Result<String, String>   a new script service {"file": path} named
//                                    after the file (free_name), started at once -> its name
//   unit_of(engine, d, store, path, last) -> Option<String>   the unit that holds the file, by the
//                                    name of its service (see there): the IDE restarts that
//                                    instead of a service of the file
//   default_path(store, name)        where "In Datei umwandeln" puts script NAME:
//                                    /root/scripts/NAME.jfs, NAME2.jfs ... when that is taken
//   convert(doc, dev, name) -> Result<String, String>   the inline script becomes the file
//                                    default_path (its source as it is, a line end at the end)
//                                    and its definition names the file instead of the source;
//                                    one step that keeps the running device (Doc::apply_kept:
//                                    the store takes the file first, then the service its new
//                                    definition and starts again from the file, its disk and
//                                    numbers stay); undo brings the inline source back
//
// A service keeps the path it names: renaming or deleting its file leaves it with a missing
// file (problems.rs says so, its next start fails).  Saving a file a running service uses
// changes nothing in the program until it starts again (scriptsvc.rs): the IDE says so.
// File times are the device's clock in seconds, as the shell's are.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::files::{FsErr, Store};
use jfn_model::{Device, Project, ServiceDef};
use jfn_sim::{Engine, UnitState};

use super::{edits, scripts, Doc};
use crate::app::tr::{t, tf};

/// the device's files as the project has them
pub(crate) fn store_of(p: &Project, dev: u32) -> Option<Store> {
    let d = p.device(dev)?;
    let m = p.model_of(dev)?;
    let mut s = Store::for_model(m);
    s.load(&d.files);
    Some(s)
}

/// a text file's content
pub(crate) fn text_of(st: &Store, path: &str) -> Option<String> {
    let b = st.read(path).ok()?;
    core::str::from_utf8(b).ok().map(String::from)
}

/// why the store refused (the end of a sentence)
fn why(e: FsErr) -> &'static str {
    match e {
        FsErr::NoSpace => t("kein Platz mehr"),
        FsErr::NotFound | FsErr::NotDir => t("das Verzeichnis fehlt"),
        FsErr::IsDir => t("dort liegt ein Verzeichnis"),
        FsErr::Exists => t("den Namen gibt es dort schon"),
        FsErr::BadName => t("der Name geht nicht"),
        FsErr::NotEmpty => t("das Verzeichnis ist nicht leer"),
        FsErr::ReadOnly => t("das gehört zum System des Geräts"),
    }
}

fn device(doc: &Doc, dev: u32) -> Result<Device, String> {
    doc.proj.device(dev).cloned().ok_or_else(|| tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()]))
}

fn store(doc: &Doc, dev: u32) -> Result<Store, String> {
    store_of(&doc.proj, dev).ok_or_else(|| String::from(t("Dieses Gerät hat keine Dateien.")))
}

/// the device's clock in seconds (a file's time)
fn now_s(doc: &Doc, dev: u32) -> i64 {
    doc.sim.engine().clock(dev).map_or(0, |c| c.div_euclid(1_000_000_000))
}

/// the store's records as one step of the project (Action::Files follows, no restart)
fn put(doc: &mut Doc, dev: u32, st: &Store, label: &str) {
    let records = st.records();
    if let Some(e) = edits::device_edit(&doc.proj, dev, label, |x| x.files = records) {
        doc.apply(e);
    }
}

/// file `path` gets `text` -> Ok(false) when it had it
pub(crate) fn save(doc: &mut Doc, dev: u32, path: &str, text: &str) -> Result<bool, String> {
    let n = save_many(doc, dev, &[(String::from(path), String::from(text))], "")?;
    Ok(n > 0)
}

/// several files in one step (label "": "Datei … speichern" for one, "… Dateien …" for more)
/// -> how many changed
pub(crate) fn save_many(doc: &mut Doc, dev: u32, files: &[(String, String)], label: &str) -> Result<usize, String> {
    let d = device(doc, dev)?;
    let mut st = store(doc, dev)?;
    let t0 = now_s(doc, dev);
    let mut changed = Vec::new();
    for (path, text) in files {
        let p = st.norm(path).map_err(|e| tf("„{}“ wurde nicht gespeichert: {}.", &[path, why(e)]))?;
        if text_of(&st, &p).as_deref() == Some(text.as_str()) {
            continue;
        }
        st.write(&p, text.as_bytes(), t0).map_err(|e| tf("„{}“ wurde nicht gespeichert: {}.", &[&p, why(e)]))?;
        changed.push(p);
    }
    if changed.is_empty() {
        return Ok(0);
    }
    let label = match (label, changed.len()) {
        ("", 1) => format!("Datei {} auf {} speichern", changed[0], d.name),
        ("", n) => format!("{} Dateien auf {} speichern", n, d.name),
        (l, _) => String::from(l),
    };
    put(doc, dev, &st, &label);
    Ok(changed.len())
}

/// a new file with `text` -> its canonical path
pub(crate) fn new_file(doc: &mut Doc, dev: u32, path: &str, text: &str) -> Result<String, String> {
    let d = device(doc, dev)?;
    let mut st = store(doc, dev)?;
    let p = st.norm(path).map_err(|e| tf("„{}“ geht nicht: {}.", &[path, why(e)]))?;
    if st.exists(&p) {
        return Err(tf("„{}“ gibt es schon.", &[&p]));
    }
    let t0 = now_s(doc, dev);
    st.write(&p, text.as_bytes(), t0).map_err(|e| tf("„{}“ geht nicht: {}.", &[&p, why(e)]))?;
    put(doc, dev, &st, &format!("Datei {} auf {} anlegen", p, d.name));
    Ok(p)
}

/// the unit file of script `exec` (workspace/ide/deploy.rs) -> its path
pub(crate) fn new_unit(doc: &mut Doc, dev: u32, name: &str, exec: &str) -> Result<String, String> {
    use jfn_model::unit;
    let d = device(doc, dev)?;
    if !unit::name_ok(name) {
        return Err(tf("„{}“ taugt nicht als Name einer Einheit.", &[name]));
    }
    let mut st = store(doc, dev)?;
    let p = unit::path_of(name);
    if st.exists(&p) {
        return Err(tf("„{}“ gibt es schon.", &[&p]));
    }
    let t0 = now_s(doc, dev);
    let text = format!("[Unit]\nDescription={}\n\n[Service]\nExecStart=jfs {}\nRestart=on-failure\nRestartSec=1\n\n[Install]\nWantedBy=multi-user.target\n", unit::stem(name), exec);
    st.mkdir(unit::UNIT_DIR, t0, true).map_err(|e| tf("„{}“ geht nicht: {}.", &[&p, why(e)]))?;
    st.write(&p, text.as_bytes(), t0).map_err(|e| tf("„{}“ geht nicht: {}.", &[&p, why(e)]))?;
    put(doc, dev, &st, &format!("Einheit {} auf {} anlegen", name, d.name));
    Ok(p)
}

/// a new folder -> its canonical path
pub(crate) fn new_dir(doc: &mut Doc, dev: u32, path: &str) -> Result<String, String> {
    let d = device(doc, dev)?;
    let mut st = store(doc, dev)?;
    let p = st.norm(path).map_err(|e| tf("„{}“ geht nicht: {}.", &[path, why(e)]))?;
    if st.exists(&p) {
        return Err(tf("„{}“ gibt es schon.", &[&p]));
    }
    let t0 = now_s(doc, dev);
    st.mkdir(&p, t0, false).map_err(|e| tf("„{}“ geht nicht: {}.", &[&p, why(e)]))?;
    put(doc, dev, &st, &format!("Ordner {} auf {} anlegen", p, d.name));
    Ok(p)
}

/// a file or folder gets another path -> the new one
pub(crate) fn rename(doc: &mut Doc, dev: u32, from: &str, to: &str) -> Result<String, String> {
    let d = device(doc, dev)?;
    let mut st = store(doc, dev)?;
    let p = st.norm(to).map_err(|e| tf("„{}“ geht nicht: {}.", &[to, why(e)]))?;
    if p == from {
        return Ok(p);
    }
    if st.exists(&p) {
        return Err(tf("„{}“ gibt es schon.", &[&p]));
    }
    st.rename(from, &p).map_err(|e| tf("„{}“ geht nicht: {}.", &[&p, why(e)]))?;
    put(doc, dev, &st, &format!("{} auf {} in {} umbenennen", from, d.name, p));
    Ok(p)
}

/// a file, or a folder with all in it, goes
pub(crate) fn remove(doc: &mut Doc, dev: u32, path: &str) -> Result<(), String> {
    let d = device(doc, dev)?;
    let mut st = store(doc, dev)?;
    let r = if st.is_dir(path) { st.remove_tree(path) } else { st.remove(path) };
    r.map_err(|e| tf("„{}“ lässt sich nicht löschen: {}.", &[path, why(e)]))?;
    put(doc, dev, &st, &format!("{} auf {} löschen", path, d.name));
    Ok(())
}

/// the file of a script service's definition
pub(crate) fn file_of(def: &ServiceDef) -> Option<&str> {
    def.config.get("file").and_then(|v| v.as_str())
}

/// the script services of device `d` that run file `path` (canonical)
pub(crate) fn bound(d: &Device, st: &Store, path: &str) -> Vec<String> {
    scripts::of(d).iter().filter(|s| file_of(s).and_then(|f| st.norm(f).ok()).as_deref() == Some(path)).map(|s| s.name.clone()).collect()
}

/// the unit that holds file `path` (canonical), by the name of its service: one that runs or waits
/// for its restart always (a service of the file would fail at the unit's port); `last`, the one
/// the IDE ran last even when stopped, and one that failed, while no script service is bound to
/// the file.  Any other unit file changes nothing: it has not started the file
pub(crate) fn unit_of(e: &Engine, d: &Device, st: &Store, path: &str, last: Option<&str>) -> Option<String> {
    let units = e.units_of_file(d.id, path);
    let free = bound(d, st, path).is_empty();
    let holds = |s: UnitState| matches!(s, UnitState::Active | UnitState::AutoRestart);
    units
        .iter()
        .find(|(n, s)| Some(n.as_str()) == last && (holds(*s) || free))
        .or_else(|| units.iter().find(|(_, s)| holds(*s)))
        .or_else(|| units.iter().find(|(_, s)| *s == UnitState::Failed && free))
        .map(|(n, _)| n.clone())
}

/// the name a file gives a script service ("server" of /root/app/server.jfs)
fn stem(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let base = base.strip_suffix(".jfs").unwrap_or(base);
    let s: String = base.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).take(28).collect();
    if s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) { s } else { String::from("script") }
}

/// a new script service for file `path` (one step; it starts at once) -> its name
pub(crate) fn bind(doc: &mut Doc, dev: u32, path: &str) -> Result<String, String> {
    let d = device(doc, dev)?;
    let name = scripts::free_name(&d, &stem(path));
    let config = Value::object([("file", Value::string(path))]);
    let def = ServiceDef { kind: String::from(scripts::KIND), name: name.clone(), disabled: false, config };
    let label = format!("Dienst {} auf {} für {} anlegen", name, d.name, path);
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| x.services.push(def)) {
        doc.apply_kept(e, dev);
    }
    Ok(name)
}

/// where "In Datei umwandeln" puts script `name`
pub(crate) fn default_path(st: &Store, name: &str) -> String {
    let base = format!("/root/scripts/{}", name);
    let p = format!("{}.jfs", base);
    if !st.exists(&p) {
        return p;
    }
    let mut k = 2;
    loop {
        let p = format!("{}{}.jfs", base, k);
        if !st.exists(&p) {
            return p;
        }
        k += 1;
    }
}

/// the inline script `name` becomes a file of the device (see the module comment) -> the file
pub(crate) fn convert(doc: &mut Doc, dev: u32, name: &str) -> Result<String, String> {
    let d = device(doc, dev)?;
    let j = scripts::find(&d, name).ok_or_else(|| tf("Das Skript „{}“ gibt es auf {} nicht.", &[name, &d.name]))?;
    if file_of(&d.services[j]).is_some() {
        return Err(tf("„{}“ läuft schon aus einer Datei.", &[name]));
    }
    let mut st = store(doc, dev)?;
    let path = default_path(&st, name);
    let t0 = now_s(doc, dev);
    let mut text = scripts::source_of(&d.services[j]);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    st.mkdir(&st.parent(&path), t0, true).map_err(|e| tf("„{}“ geht nicht: {}.", &[&path, why(e)]))?;
    st.write(&path, text.as_bytes(), t0).map_err(|e| tf("„{}“ geht nicht: {}.", &[&path, why(e)]))?;
    let mut config = match &d.services[j].config {
        Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| k != "source").cloned().collect()),
        _ => Value::Object(Vec::new()),
    };
    config.insert("file", Value::string(&path));
    let records = st.records();
    let label = format!("Skript {} auf {} in die Datei {} umwandeln", name, d.name, path);
    if let Some(e) = edits::device_edit(&doc.proj, dev, &label, |x| {
        x.files = records;
        x.services[j].config = config;
    }) {
        doc.apply_kept(e, dev);
    }
    Ok(path)
}
