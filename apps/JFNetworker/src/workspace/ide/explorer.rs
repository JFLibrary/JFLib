// explorer -- the IDE's Explorer: the device's files as a tree (folders first, then the files by
// name; .jfs files are the IDE's own, other text files open too), above them the device's
// scripts whose source is still inline in their definition ("Eingebettete Skripte") until they
// are converted; Neue Datei, Neuer Ordner, Umbenennen, Löschen and In Datei umwandeln are undo
// steps of the project (doc/ide.rs, doc/scripts.rs)
//
//   view(n, ui, k, id, cx, acts)   the buttons and the tree; a double click or Enter opens a file
//                                  or an inline script, Entf deletes, F2 renames (the name
//                                  flyout hangs at Neue Datei)
//   chosen_dir(n, k, st)           where new files go: the chosen folder, the chosen file's, else
//                                  /root
//   ask(n, ui, k, what)            opens the name flyout (the side bar shows the Explorer)
//   make(n, ui, k, cx, what, name) / delete(n, k, cx, key) / convert(n, k, cx, name)
//                                  the steps; the tabs, drafts and breakpoints go with a renamed
//                                  file and from an inline script to its file
//
// A row has "●" after its name while its draft has changes (as its tab), its detail names the
// services that run the file.  /root and the folders of open files are open at first.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::Store;
use ui::*;

use super::{Act, AskFor, Cx, Key, SIDE_EXPLORER};
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{ide as dide, scripts};

/// what a row of the tree is
#[derive(Clone, Debug, PartialEq, Eq)]
enum What {
    Dir(String),
    File(String),
    Inlines,
    Inline(String),
}

/// a row of the tree
struct Node {
    key: u64,
    depth: u16,
    dir: bool,
    label: String,
    detail: String,
    what: What,
}

/// a key from a text (FNV-1a)
fn key_of(s: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h
}

const INLINES: &str = "\u{0}inline";

/// the rows as they show (closed folders hide what is in them)
fn nodes(n: &Net, cx: &Cx, open: &TreeState) -> Vec<Node> {
    let mut v = Vec::new();
    let changed = |key: &Key| n.devwin.ide.drafts.get(&(cx.dev, key.clone())).is_some_and(|d| d.changed());
    let inl: Vec<String> = scripts::of(&cx.d).iter().filter(|s| dide::file_of(s).is_none()).map(|s| s.name.clone()).collect();
    if !inl.is_empty() {
        let gk = key_of(INLINES);
        v.push(Node { key: gk, depth: 0, dir: true, label: String::from(t("Eingebettete Skripte")), detail: String::new(), what: What::Inlines });
        if open.expanded.contains(&gk) {
            for name in inl {
                let ch = changed(&Key::Inline(name.clone()));
                let label = if ch { format!("{} ●", name) } else { name.clone() };
                v.push(Node { key: key_of(&format!("{}/{}", INLINES, name)), depth: 1, dir: false, label, detail: String::new(), what: What::Inline(name) });
            }
        }
    }
    fn walk(n: &Net, cx: &Cx, open: &TreeState, dir: &str, depth: u16, v: &mut Vec<Node>) {
        let mut items = cx.st.list(dir).unwrap_or_default();
        items.sort_by(|a, b| (!a.1.dir, &a.0).cmp(&(!b.1.dir, &b.0)));
        for (name, s) in items {
            let path = if dir == "/" { format!("/{}", name) } else { format!("{}/{}", dir, name) };
            let key = key_of(&path);
            if s.dir {
                v.push(Node { key, depth, dir: true, label: name, detail: String::new(), what: What::Dir(path.clone()) });
                if open.expanded.contains(&key) && v.len() < 2000 {
                    walk(n, cx, open, &path, depth + 1, v);
                }
            } else {
                let ch = n.devwin.ide.drafts.get(&(cx.dev, Key::File(path.clone()))).is_some_and(|d| d.changed());
                let b = dide::bound(&cx.d, &cx.st, &path);
                let detail = if b.is_empty() { String::new() } else { tf("Dienst {}", &[&b.join(", ")]) };
                let label = if ch { format!("{} ●", name) } else { name };
                v.push(Node { key, depth, dir: false, label, detail, what: What::File(path) });
            }
        }
    }
    walk(n, cx, open, "/", 0, &mut v);
    v
}

/// where new files go (see the module comment)
pub(crate) fn chosen_dir(n: &mut Net, k: usize, st: &Store) -> String {
    let Some(w) = n.devwin.get(k) else { return String::from("/root") };
    let cur = w.ide.tree.cur;
    let paths: Vec<(String, bool)> = {
        let mut v = Vec::new();
        let mut todo = alloc::vec![String::from("/")];
        while let Some(d) = todo.pop() {
            for (name, s) in st.list(&d).unwrap_or_default() {
                let p = if d == "/" { format!("/{}", name) } else { format!("{}/{}", d, name) };
                if Some(key_of(&p)) == cur {
                    v.push((p.clone(), s.dir));
                }
                if s.dir {
                    todo.push(p);
                }
            }
        }
        v
    };
    match paths.first() {
        Some((p, true)) => p.clone(),
        Some((p, false)) => st.parent(p),
        None => String::from("/root"),
    }
}

/// opens the name flyout for `what`
pub(crate) fn ask(n: &mut Net, ui: &mut Ui, k: usize, what: AskFor) {
    let text = match &what {
        AskFor::NewFile(_) => String::from("neu.jfs"),
        AskFor::NewDir(_) => String::from(t("ordner")),
        AskFor::Rename(p) => String::from(p.rsplit('/').next().unwrap_or(p)),
    };
    let id = super::id_of(k);
    if let Some(w) = n.devwin.get(k) {
        w.ide.side_on = true;
        w.ide.side = SIDE_EXPLORER;
        w.ide.ask = Some((what, text, None));
    }
    // (the drop-down button opened it already when it was clicked)
    if !ui.popup_is_open(id + 0xC0) {
        ui.popup_open(id + 0xC0);
    }
    ui.focus(id + 0xC1);
}

/// the Explorer (see the module comment)
pub(crate) fn view(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    // /root, the inline scripts and the folders of open tabs are open at first
    let tabs: Vec<Key> = n.devwin.get(k).map(|w| w.ide.tabs.clone()).unwrap_or_default();
    let Some(w) = n.devwin.get(k) else { return };
    if !w.ide.tree_init {
        w.ide.tree_init = true;
        w.ide.tree.expanded.insert(key_of("/root"));
        w.ide.tree.expanded.insert(key_of(INLINES));
        for key in &tabs {
            if let Key::File(p) = key {
                let mut d = String::new();
                for part in p.split('/').filter(|x| !x.is_empty()).collect::<Vec<_>>().split_last().map_or(&[][..], |x| x.1) {
                    d.push('/');
                    d.push_str(part);
                    w.ide.tree.expanded.insert(key_of(&d));
                }
            }
        }
    }
    let tree = w.ide.tree.clone();
    let rows = nodes(n, cx, &tree);
    let cur = tree.cur.and_then(|c| rows.iter().find(|r| r.key == c)).map(|r| r.what.clone());
    let dir = chosen_dir(n, k, &cx.st);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 2);
    ui.next_icon(IC_ADD);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY);
    let ev = ui.dropdown_button(id + 0xC0, t("Neue Datei …"));
    if ev & EV_OPEN != 0 && n.devwin.get(k).is_some_and(|w| w.ide.ask.is_none()) {
        ask(n, ui, k, AskFor::NewFile(dir.clone()));
    }
    ui.tooltip(t("Neue Datei im gewählten Ordner"));
    ask_flyout(n, ui, k, id, acts);
    ui.focus_group_begin(id + 0x11F);
    let mut button = |ui: &mut Ui, bid: u32, icon: u32, label: &str, tip: &str, on: bool, a: Act| {
        ui.next_icon(icon);
        ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY | if on { 0 } else { ST_DISABLED });
        if ui.button(bid, label) & EV_ACT != 0 {
            acts.push(a);
        }
        ui.tooltip(tip);
    };
    button(ui, id + 0x110, IC_NEW_FOLDER, t("Neuer Ordner …"), t("Neuer Ordner im gewählten Ordner"), true, Act::Ask(AskFor::NewDir(dir.clone())));
    let path = match &cur {
        Some(What::File(p)) | Some(What::Dir(p)) => Some(p.clone()),
        _ => None,
    };
    let fixed = path.as_ref().is_some_and(|p| cx.st.stat(p).is_some_and(|s| s.builtin));
    button(ui, id + 0x111, IC_EDIT, t("Umbenennen …"), t("Umbenennen (F2)"), path.is_some() && !fixed, Act::Ask(AskFor::Rename(path.clone().unwrap_or_default())));
    let del = match &cur {
        Some(What::File(p)) | Some(What::Dir(p)) if !fixed => Some(Key::File(p.clone())),
        Some(What::Inline(name)) => Some(Key::Inline(name.clone())),
        _ => None,
    };
    button(ui, id + 0x112, IC_DELETE, t("Löschen"), t("Löschen (Entf) – ein Rückgängig-Schritt"), del.is_some(), Act::Delete(del.clone().unwrap_or(Key::File(String::new()))));
    let inl = match &cur {
        Some(What::Inline(name)) => Some(name.clone()),
        _ => None,
    };
    button(ui, id + 0x113, IC_SAVE_AS, t("In Datei umwandeln"), t("In Datei umwandeln: das Skript kommt als Datei auf das Gerät, der Dienst startet von nun an aus ihr"), inl.is_some(), Act::Convert(inl.clone().unwrap_or_default()));
    ui.focus_group_end();
    ui.box_end();
    let trows: Vec<TreeRow> = rows
        .iter()
        .map(|r| TreeRow {
            key: r.key,
            depth: r.depth,
            has_children: r.dir,
            check: None,
            icon: match (&r.what, r.dir) {
                (What::Inlines, _) => IC_LAYERS,
                (_, true) if tree.expanded.contains(&r.key) => IC_FOLDER_OPEN,
                (_, true) => IC_FOLDER,
                (What::Inline(_), _) => IC_PAGE,
                (What::File(p), _) if p.ends_with(".jfs") => IC_DOCUMENT,
                _ => IC_PAGE,
            },
            label: &r.label,
            detail: if r.detail.is_empty() { None } else { Some(&r.detail) },
        })
        .collect();
    let Some(w) = n.devwin.get(k) else {
        ui.box_end();
        return;
    };
    ui.next_size(-1, -1);
    let tev = ui.tree(id + 0x800, &mut w.ide.tree, &trows, 24);
    if let Some(a) = tev.activated.and_then(|key| rows.iter().find(|r| r.key == key)) {
        match &a.what {
            What::File(p) => acts.push(Act::Open(Key::File(p.clone()))),
            What::Inline(name) => acts.push(Act::Open(Key::Inline(name.clone()))),
            _ => {}
        }
    }
    // the tree's keys: Entf deletes, F2 renames
    if ui.focused() == id + 0x800 {
        if let Some(d) = &del
            && ui.capture_shortcut(VK_DELETE, 0)
        {
            acts.push(Act::Delete(d.clone()));
        }
        if let Some(p) = &path
            && !fixed
            && ui.capture_shortcut(VK_F2, 0)
        {
            acts.push(Act::Ask(AskFor::Rename(p.clone())));
        }
    }
    ui.box_end();
}

/// the name flyout of Neue Datei, Neuer Ordner and Umbenennen
fn ask_flyout(n: &mut Net, ui: &mut Ui, k: usize, id: u32, acts: &mut Vec<Act>) {
    let fid = id + 0xC0;
    if !ui.popup_is_open(fid)
        && let Some(w) = n.devwin.get(k)
    {
        w.ide.ask = None;
        w.ide.fly[0] = 0;
    }
    let Some(w) = n.devwin.get(k) else { return };
    let mut done = 0; // 1 OK, 2 Abbrechen
    if ui.flyout_begin(fid, FL_BELOW)
        && let Some((what, text, refusal)) = w.ide.ask.as_mut()
    {
        let title = match what {
            AskFor::NewFile(d) => tf("Neue Datei in {}", &[d]),
            AskFor::NewDir(d) => tf("Neuer Ordner in {}", &[d]),
            AskFor::Rename(p) => tf("Neuer Name für {}", &[p]),
        };
        ui.box_begin(0, BOX_V | pad(8), 6);
        ui.text(TS_BODY_STRONG, &title);
        ui.next_size(260, 0);
        let tev = ui.textbox(id + 0xC1, None, text, 255);
        w.ide.fly[0] += 1;
        if tev & EV_COMMIT != 0 && ui.focused() == id + 0xC1 && w.ide.fly[0] > 1 {
            done = 1;
        }
        if let Some(m) = refusal.as_ref() {
            crate::workspace::error_text(ui, m, 260);
        }
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.accent_button(id + 0xC2, t("OK")) & EV_ACT != 0 {
            done = 1;
        }
        if ui.button(id + 0xC3, t("Abbrechen")) & EV_ACT != 0 {
            done = 2;
        }
        ui.box_end();
        ui.box_end();
    }
    ui.flyout_end();
    match done {
        1 => {
            if let Some((what, text, _)) = w.ide.ask.clone() {
                acts.push(Act::Make(what, text));
            }
        }
        2 => {
            w.ide.ask = None;
            ui.popup_close();
        }
        _ => {}
    }
}

/// the step of the name flyout: a new file (it opens), a new folder (it opens in the tree), a
/// new name; a refusal stays in the flyout
pub(crate) fn make(n: &mut Net, ui: &mut Ui, k: usize, cx: &Cx, what: AskFor, name: &str) -> Result<(), String> {
    let name = name.trim();
    let r = if name.is_empty() {
        Err(String::from(t("Ein Name fehlt.")))
    } else {
        match &what {
            AskFor::NewFile(dir) => dide::new_file(&mut n.doc, cx.dev, &super::resolve(dir, name), "").map(|p| (Some(p), None)),
            AskFor::NewDir(dir) => dide::new_dir(&mut n.doc, cx.dev, &super::resolve(dir, name)).map(|p| (None, Some(p))),
            AskFor::Rename(from) => {
                let to = super::resolve(&cx.st.parent(from), name);
                dide::rename(&mut n.doc, cx.dev, from, &to).map(|to| {
                    moved(n, cx.dev, from, &to);
                    (None, None)
                })
            }
        }
    };
    match r {
        Ok((file, dir)) => {
            if let Some(w) = n.devwin.get(k) {
                w.ide.ask = None;
                for p in file.iter().chain(dir.iter()) {
                    // its folder (and a new folder itself) opens in the tree, the row is chosen
                    let parent = cx.st.parent(p);
                    w.ide.tree.expanded.insert(key_of(&parent));
                    w.ide.tree.cur = Some(key_of(p));
                }
                if let Some(d) = &dir {
                    w.ide.tree.expanded.insert(key_of(d));
                }
            }
            ui.popup_close();
            if let Some(p) = file {
                // (the new file is in the project now; the frame's store is older)
                let mut cx2 = Cx { dev: cx.dev, d: cx.d.clone(), st: cx.st.clone(), cur: cx.cur.clone(), target: cx.target.clone(), view: None, paused: None, busy: cx.busy, term: cx.term };
                if let Some(st) = dide::store_of(&n.doc.proj, cx.dev) {
                    cx2.st = st;
                }
                return super::editor::open(n, ui, k, &cx2, Key::File(p), Some((1, 1)));
            }
            Ok(())
        }
        Err(m) => {
            if let Some(w) = n.devwin.get(k)
                && let Some(a) = w.ide.ask.as_mut()
            {
                a.2 = Some(m);
                return Ok(());
            }
            Err(m)
        }
    }
}

/// a new .jfs file in `dir` from template i of doc/scripts.rs (its name after the template's,
/// a free one), opened
pub(crate) fn from_template(n: &mut Net, ui: &mut Ui, k: usize, cx: &Cx, dir: &str, i: usize) -> Result<(), String> {
    let Some(tpl) = scripts::TEMPLATES.get(i) else { return Ok(()) };
    let free = |p: &str| !cx.st.exists(p);
    let mut path = super::resolve(dir, &format!("{}.jfs", tpl.name));
    let mut no = 2;
    while !free(&path) {
        path = super::resolve(dir, &format!("{}{}.jfs", tpl.name, no));
        no += 1;
    }
    let mut text = scripts::template(i, &t).join("\n");
    text.push('\n');
    let p = dide::new_file(&mut n.doc, cx.dev, &path, &text)?;
    if let Some(w) = n.devwin.get(k) {
        w.ide.tree.expanded.insert(key_of(dir));
        w.ide.tree.cur = Some(key_of(&p));
    }
    let mut cx2 = Cx { dev: cx.dev, d: cx.d.clone(), st: cx.st.clone(), cur: cx.cur.clone(), target: cx.target.clone(), view: None, paused: None, busy: cx.busy, term: cx.term };
    if let Some(st) = dide::store_of(&n.doc.proj, cx.dev) {
        cx2.st = st;
    }
    super::editor::open(n, ui, k, &cx2, Key::File(p), Some((1, 1)))
}

/// a file or folder `from` is `to` now: the drafts, tabs and breakpoints go with it
fn moved(n: &mut Net, dev: u32, from: &str, to: &str) {
    let map = |p: &str| -> Option<String> {
        if p == from {
            Some(String::from(to))
        } else {
            p.strip_prefix(from).filter(|r| r.starts_with('/')).map(|r| format!("{}{}", to, r))
        }
    };
    let keys: Vec<(u32, Key)> = n.devwin.ide.drafts.keys().filter(|(dv, k_)| *dv == dev && matches!(k_, Key::File(p) if map(p).is_some())).cloned().collect();
    for (dv, key) in keys {
        if let (Key::File(p), Some(dr)) = (&key, n.devwin.ide.drafts.remove(&(dv, key.clone()))) {
            let np = map(p).unwrap_or_default();
            n.devwin.ide.drafts.insert((dv, Key::File(np.clone())), dr);
            if let Some(b) = n.devwin.ide.bps.remove(&(dv, key.clone())) {
                n.devwin.ide.bps.insert((dv, Key::File(np)), b);
            }
        }
    }
    for k in 0..crate::app::constants::DEVWINS {
        if let Some(w) = n.devwin.get(k)
            && w.ide.dev == dev
        {
            for t_ in w.ide.tabs.iter_mut() {
                if let Key::File(p) = t_
                    && let Some(np) = map(p)
                {
                    *p = np;
                }
            }
        }
    }
}

/// Löschen: a file or a folder with all in it, or an inline script (its service goes)
pub(crate) fn delete(n: &mut Net, k: usize, cx: &Cx, key: &Key) -> Result<(), String> {
    match key {
        Key::File(p) if p.is_empty() => Ok(()),
        Key::File(p) => dide::remove(&mut n.doc, cx.dev, p),
        Key::Inline(name) => {
            let r = scripts::remove(&mut n.doc, cx.dev, name);
            if r.is_ok() {
                n.devwin.ide.drafts.remove(&(cx.dev, key.clone()));
            }
            r
        }
    }
    .map(|_| {
        if let Some(w) = n.devwin.get(k) {
            w.ide.tree.cur = None;
        }
    })
}

/// In Datei umwandeln: the inline script becomes a file (doc/ide.rs convert); its tab, draft and
/// breakpoints go to the file
pub(crate) fn convert(n: &mut Net, k: usize, cx: &Cx, name: &str) -> Result<(), String> {
    let ik = (cx.dev, Key::Inline(String::from(name)));
    if n.devwin.ide.drafts.get(&ik).is_some_and(|d| d.changed()) {
        return Err(tf("„{}“ hat ungespeicherte Änderungen: erst speichern (Strg+S), dann umwandeln.", &[name]));
    }
    let path = dide::convert(&mut n.doc, cx.dev, name)?;
    let fk = (cx.dev, Key::File(path.clone()));
    let text = dide::store_of(&n.doc.proj, cx.dev).and_then(|st| dide::text_of(&st, &path)).unwrap_or_default();
    n.devwin.ide.drafts.remove(&ik);
    n.devwin.ide.drafts.insert(fk.clone(), super::editor::Draft::new(&text, true));
    if let Some(b) = n.devwin.ide.bps.remove(&ik) {
        n.devwin.ide.bps.insert(fk, b);
    }
    for kk in 0..crate::app::constants::DEVWINS {
        if let Some(w) = n.devwin.get(kk)
            && w.ide.dev == cx.dev
        {
            for t_ in w.ide.tabs.iter_mut() {
                if *t_ == Key::Inline(String::from(name)) {
                    *t_ = Key::File(path.clone());
                }
            }
            if w.ide.target.is_none() {
                w.ide.target = Some(String::from(name));
            }
        }
    }
    if let Some(w) = n.devwin.get(k) {
        let parent = cx.st.parent(&path);
        w.ide.tree.expanded.insert(key_of("/root"));
        w.ide.tree.expanded.insert(key_of(&parent));
        w.ide.tree.cur = Some(key_of(&path));
        w.ide.msg = None;
    }
    Ok(())
}
