// design_store -- (JD11) the user's saved designs (TH-03): Entwurf › Designs › ▾ › Aktuelles
// Design speichern
//
//   load(app)              once: the saved designs into app.designs.saved
//   ask_save(app, ui)      opens the dialog that asks for a name
//   dialog(app, ui)        the dialog (every pass): the design of the page shown is saved under
//                          the name, with a scheme number of its own (61001 ...)
//   delete_all(app)        forget them
//
// Where: the settings keep the list ([designs] count, name1, id1 ...), the theme parts lie
// beside them as config://design<id>.xml.  A run without a window (scripts) keeps both in
// memory only.  The part saved is the page's theme part with every schemeEnum set to the new
// number and the new name in a:theme@name; JFDiagrammer shows it under "Gespeichert".

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply;
use ui::i18n::t;
use ui::*;

use crate::design_gallery::{Entry, Kind};
use crate::Diagrammer;

const SEC: &str = "designs";
const FIRST: u32 = 61001;
const ID_DLG: u32 = 0x5D60_0000;

/// the dialog's state
pub(crate) struct SaveDialog {
    pub open: Shared<i32>,
    pub name: String,
}

impl Default for SaveDialog {
    fn default() -> SaveDialog {
        SaveDialog { open: Shared::new(0), name: String::new() }
    }
}

fn path(id: u32) -> String {
    format!("config://design{id}.xml")
}

/// the saved designs, once
pub(crate) fn load(app: &mut Diagrammer) {
    if app.designs.saved_loaded {
        return;
    }
    app.designs.saved_loaded = true;
    let ini = app.opts.ini();
    let n: u32 = ini.get(SEC, "count").and_then(|v| v.parse().ok()).unwrap_or(0);
    for k in 1..=n.min(64) {
        let (Some(name), Some(id)) = (ini.get(SEC, &format!("name{k}")), ini.get(SEC, &format!("id{k}")).and_then(|v| v.parse::<u32>().ok())) else { continue };
        let Some(data) = plat::read_file(&path(id)) else { continue };
        if apply::part_id(&data) != Some(id) {
            continue;
        }
        app.designs.saved.push(Entry { id, name: String::from(name), part: Rc::new(data), kind: Kind::Saved });
    }
}

fn store(app: &mut Diagrammer, ui: &Ui) {
    let list: Vec<(String, u32)> = app.designs.saved.iter().map(|e| (e.name.clone(), e.id)).collect();
    let ini = app.opts.ini_mut();
    ini.set(SEC, "count", &format!("{}", list.len()));
    for (k, (name, id)) in list.iter().enumerate() {
        ini.set(SEC, &format!("name{}", k + 1), name);
        ini.set(SEC, &format!("id{}", k + 1), &format!("{id}"));
    }
    app.opts.save();
    if !ui.headless() {
        for e in &app.designs.saved {
            let _ = plat::write_file(&path(e.id), &e.part);
        }
    }
}

// the XML of a theme part with every schemeEnum = `id` and the theme's name `name`
pub(crate) fn retag(part: &[u8], id: u32, name: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(part);
    let mut out = String::with_capacity(text.len());
    let mut rest: &str = &text;
    while let Some(p) = rest.find("schemeEnum=\"") {
        let start = p + "schemeEnum=\"".len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest.find('"').unwrap_or(0);
        out.push_str(&format!("{id}"));
        rest = &rest[end..];
    }
    out.push_str(rest);
    // the theme's name: the first name attribute of a:theme
    let esc: String = name.chars().map(|c| match c {
        '&' => String::from("&amp;"),
        '<' => String::from("&lt;"),
        '"' => String::from("&quot;"),
        c => String::from(c),
    }).collect();
    if let Some(th) = out.find("<a:theme ") {
        if let Some(n) = out[th..].find(" name=\"") {
            let s = th + n + " name=\"".len();
            if let Some(e) = out[s..].find('"') {
                out.replace_range(s..s + e, &esc);
            }
        }
    }
    out.into_bytes()
}

/// open the dialog for the design of the page shown
pub(crate) fn ask_save(app: &mut Diagrammer, ui: &mut Ui) {
    let name = crate::design_gallery::current(app)
        .and_then(|(id, _)| crate::design_gallery::entries(app).into_iter().find(|e| e.id == id))
        .map_or(String::new(), |e| format!("{} {}", e.name, t("(eigenes)")));
    app.design_save.name = name;
    app.design_save.open.set(1);
    ui.repass();
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.design_save.open.clone();
    if !ui.dialog_begin(ID_DLG, t("Aktuelles Design speichern"), &open) {
        return;
    }
    ui.next_size(360, 0);
    ui.text(TS_BODY | TX_WRAP, t("Das Design der angezeigten Seite wird unter diesem Namen gespeichert und erscheint im Katalog unter „Gespeichert“."));
    ui.spacer(8);
    ui.next_size(360, 0);
    let mut name = core::mem::take(&mut app.design_save.name);
    ui.textbox(ID_DLG + 1, Some(t("Name")), &mut name, 64);
    let ok = !name.trim().is_empty();
    let r = ui.dialog_buttons(Some(t("Speichern")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    app.design_save.name = name;
    if r == DLG_PRIMARY && ok {
        save(app, ui);
    }
    if r != DLG_NONE {
        app.design_save.open.set(0);
    }
}

fn save(app: &mut Diagrammer, ui: &mut Ui) {
    load(app);
    let Some((id, _)) = crate::design_gallery::current(app) else { return };
    let Some(e) = crate::design_gallery::entries(app).into_iter().find(|e| e.id == id) else { return };
    let new_id = (FIRST..).find(|n| !app.designs.saved.iter().any(|s| s.id == *n)).unwrap_or(FIRST);
    let name = String::from(app.design_save.name.trim());
    let part = retag(&e.part, new_id, &name);
    app.designs.saved.push(Entry { id: new_id, name, part: Rc::new(part), kind: Kind::Saved });
    store(app, ui);
    ui.repass();
}

/// forget the saved designs
pub(crate) fn delete_all(app: &mut Diagrammer) {
    load(app);
    let n = app.designs.saved.len();
    app.designs.saved.clear();
    let ini = app.opts.ini_mut();
    for k in 1..=n {
        ini.remove(SEC, &format!("name{k}"));
        ini.remove(SEC, &format!("id{k}"));
    }
    ini.set(SEC, "count", "0");
    app.opts.save();
}
