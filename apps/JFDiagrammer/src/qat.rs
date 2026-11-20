// qat -- (JD8 rest) the quick access toolbar in the title bar (UI-30) and its customization (the
// command Symbolleiste für den Schnellzugriff anpassen, UI-19's quick access part)
//
//   buttons(app, ui)          the toolbar's buttons (key tips 1 .. 9), Rückgängig and
//                             Wiederholen naming the step they take ("Rückgängig: Verschieben")
//   more(app, ui)             the ▾ button with its menu: the common commands to tick on and off,
//                             Weitere Befehle …
//   dialog(app, ui)           every pass: Weitere Befehle: every command with an icon, the
//                             toolbar's list, Hinzufügen, Entfernen, Nach oben, Nach unten,
//                             Zurücksetzen
//
// The toolbar is kept in the settings as command names (the constants of commands.rs, as the own
// shortcuts are): [qat] items=SAVE,UNDO,REDO.  Office's other choices (below the ribbon, import
// and export) are left out.

use alloc::string::String;
use alloc::vec::Vec;

use ui::i18n::t;
use ui::*;

use crate::commands::{self, DEFS, QAT_CUSTOMIZE, REDO, UNDO};
use crate::icons::JD_QAT_CUSTOMIZE;
use crate::Diagrammer;

const ID_MORE: u32 = 120;
const ID_MENU: u32 = 0x5C20_0000;
const ID_DLG: u32 = 0x5C30_0000;

/// the toolbar as Office starts it
const DEFAULT: [&str; 3] = ["SAVE", "UNDO", "REDO"];
/// the common commands the menu offers to tick (Office's list, those JFDiagrammer has)
const COMMON: [&str; 9] = ["NEW", "OPEN", "SAVE", "PRINT", "UNDO", "REDO", "CUT", "COPY", "PASTE"];

/// the dialog's state
#[derive(Default)]
pub(crate) struct QatState {
    pub dlg: Shared<i32>,
    items: Vec<u32>,
    pick_all: i32,
    pick_mine: i32,
}

fn id_of(name: &str) -> Option<u32> {
    DEFS.iter().find(|d| d.name == name).map(|d| d.id)
}

fn name_of(id: u32) -> &'static str {
    DEFS.iter().find(|d| d.id == id).map_or("", |d| d.name)
}

/// the toolbar's commands (from the settings, else Office's three)
pub(crate) fn items(app: &Diagrammer) -> Vec<u32> {
    match app.opts.ini().get("qat", "items") {
        Some(v) => v.split(',').filter_map(|n| id_of(n.trim())).filter(|&id| commands::def(id).is_some_and(|d| d.icon != 0)).collect(),
        None => DEFAULT.iter().filter_map(|n| id_of(n)).collect(),
    }
}

fn store(app: &mut Diagrammer, list: &[u32]) {
    let text: Vec<&str> = list.iter().map(|&id| name_of(id)).collect();
    app.opts.ini_mut().set("qat", "items", &text.join(","));
    app.opts.save();
}

// a button's QuickInfo: Rückgängig and Wiederholen name their step
fn tip(app: &Diagrammer, ui: &Ui, id: u32) -> String {
    let step = app.opened.as_ref().and_then(|o| match id {
        UNDO => o.ed.history.undo_label(),
        REDO => o.ed.history.redo_label(),
        _ => None,
    });
    match step {
        Some(s) if !s.is_empty() => {
            let full = commands::tip(ui, id);
            let label = commands::label(id);
            full.replacen(label, &alloc::format!("{label}: {s}"), 1)
        }
        _ => commands::tip(ui, id),
    }
}

/// the toolbar's buttons
pub(crate) fn buttons(app: &mut Diagrammer, ui: &mut Ui) {
    const KEYS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
    for (k, id) in items(app).into_iter().enumerate() {
        ui.next_size(28, 26);
        ui.cmd_button(id, ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
        ui.tooltip(&tip(app, ui, id));
        if let Some(key) = KEYS.get(k) {
            ui.keytip_keys(1, key);
        }
    }
}

/// the ▾ button and its menu
pub(crate) fn more(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(22, 26);
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
    ui.next_icon(JD_QAT_CUSTOMIZE);
    if ui.button(ID_MORE, commands::label(QAT_CUSTOMIZE)) & EV_ACT != 0 {
        ui.popup_open(ID_MENU);
    }
    ui.tooltip(&commands::tip(ui, QAT_CUSTOMIZE));
    if ui.flyout_begin(ID_MENU, 0) {
        ui.text(TS_BODY_STRONG, commands::label(QAT_CUSTOMIZE));
        let mut list = items(app);
        for (k, name) in COMMON.iter().enumerate() {
            let Some(id) = id_of(name) else { continue };
            let on = list.contains(&id);
            ui.next_flags(if on { ST_CHECKED } else { 0 });
            if ui.menu_item(ID_MENU + 1 + k as u32, commands::label(id)) & EV_ACT != 0 {
                if on {
                    list.retain(|&x| x != id);
                } else {
                    list.push(id);
                }
                store(app, &list);
                ui.repass();
            }
        }
        ui.menu_sep();
        if ui.menu_item(ID_MENU + 0x20, &alloc::format!("{}…", t("Weitere Befehle"))) & EV_ACT != 0 {
            open_dialog(app, ui);
        }
    }
    ui.flyout_end();
}

/// Symbolleiste für den Schnellzugriff anpassen (the command): its menu at the ▾ button
pub(crate) fn open_menu(_app: &mut Diagrammer, ui: &mut Ui) {
    ui.popup_open(ID_MENU);
    ui.repass();
}

fn open_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    app.qat.items = items(app);
    app.qat.pick_all = 0;
    app.qat.pick_mine = if app.qat.items.is_empty() { -1 } else { 0 };
    app.qat.dlg.set(1);
    ui.popup_close();
    ui.repass();
}

// the commands the toolbar can take: those with an icon, by label
fn choices() -> Vec<u32> {
    let mut v: Vec<u32> = DEFS.iter().filter(|d| d.icon != 0).map(|d| d.id).collect();
    v.sort_by(|a, b| commands::label(*a).cmp(commands::label(*b)));
    v.dedup_by(|a, b| commands::label(*a) == commands::label(*b));
    v
}

// a list of commands with their icons
fn list(ui: &mut Ui, id: u32, pick: &mut i32, ids: &[u32]) {
    ui.next_size(-1, 260);
    let (first, end) = ui.list_begin(id, pick, ids.len() as i32, 26);
    for i in first..end {
        let c = ids[i as usize];
        if let Some(d) = commands::def(c) {
            ui.next_icon(d.icon);
        }
        ui.list_item(i, commands::label(c));
    }
    ui.list_end(pick);
}

/// every pass: the dialog Weitere Befehle
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.qat.dlg.clone();
    if !ui.dialog_begin(ID_DLG, commands::label(QAT_CUSTOMIZE), &dlg) {
        return;
    }
    let all = choices();
    let st = &mut app.qat;
    ui.next_size(620, 0);
    ui.box_begin(ID_DLG + 1, BOX_H, 12);
    ui.next_size(260, 0);
    ui.box_begin(0, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Befehle"));
    list(ui, ID_DLG + 0x100, &mut st.pick_all, &all);
    ui.box_end();
    ui.next_size(100, 0);
    ui.box_begin(0, BOX_V, 6);
    ui.spacer(26);
    let chosen = usize::try_from(st.pick_all).ok().and_then(|k| all.get(k).copied());
    ui.next_size(-1, 0);
    ui.next_flags(if chosen.is_some_and(|c| !st.items.contains(&c)) { 0 } else { ST_DISABLED });
    if ui.button(ID_DLG + 2, t("Hinzufügen")) & EV_ACT != 0 {
        if let Some(c) = chosen.filter(|c| !st.items.contains(c)) {
            st.items.push(c);
            st.pick_mine = st.items.len() as i32 - 1;
        }
    }
    let mine = usize::try_from(st.pick_mine).ok().filter(|&k| k < st.items.len());
    ui.next_size(-1, 0);
    ui.next_flags(if mine.is_some() { 0 } else { ST_DISABLED });
    if ui.button(ID_DLG + 3, t("Entfernen")) & EV_ACT != 0 {
        if let Some(k) = mine {
            st.items.remove(k);
            st.pick_mine = (k as i32).min(st.items.len() as i32 - 1);
        }
    }
    ui.next_size(-1, 0);
    ui.next_flags(if mine.is_some_and(|k| k > 0) { 0 } else { ST_DISABLED });
    if ui.button(ID_DLG + 4, t("Nach oben")) & EV_ACT != 0 {
        if let Some(k) = mine.filter(|&k| k > 0) {
            st.items.swap(k, k - 1);
            st.pick_mine -= 1;
        }
    }
    ui.next_size(-1, 0);
    ui.next_flags(if mine.is_some_and(|k| k + 1 < st.items.len()) { 0 } else { ST_DISABLED });
    if ui.button(ID_DLG + 5, t("Nach unten")) & EV_ACT != 0 {
        if let Some(k) = mine.filter(|&k| k + 1 < st.items.len()) {
            st.items.swap(k, k + 1);
            st.pick_mine += 1;
        }
    }
    ui.spacer(12);
    ui.next_size(-1, 0);
    if ui.button(ID_DLG + 6, t("Zurücksetzen")) & EV_ACT != 0 {
        st.items = DEFAULT.iter().filter_map(|n| id_of(n)).collect();
        st.pick_mine = 0;
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Symbolleiste für den Schnellzugriff"));
    let items = st.items.clone();
    list(ui, ID_DLG + 0x200, &mut st.pick_mine, &items);
    ui.box_end();
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        let list = app.qat.items.clone();
        store(app, &list);
    }
    if r != DLG_NONE {
        app.qat.dlg.set(0);
        ui.repass();
    }
}
