// settings -- the dialogs of Werkzeuge › Einstellungen (Ctrl+,) and Hilfe › Tastenkürzel (F1)
//
//   dialog(n, ui)       the user's settings: the page ui.settings_page builds from
//                       app::prefs' schema in a scroll view; a change takes effect at once
//                       (grid and its step -- the open project takes the step --, snapping,
//                       minimap, the speed, motion, language, the autosave interval) and the
//                       file is written when the dialog closes; below it the answers to the
//                       program's questions (ui.permissions_page)
//   shortcuts(n, ui)    every key of the app by area: the commands with their accelerators
//                       (from the command table, in the current language) and the keys of the
//                       canvas, the consoles, the simulation, the window, the IDE and a
//                       host's desktop (F11, the keys of its windows)
//
// Both are dialogs of the main window; Esc and Schließen end them.

use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::app::tr::t;
use crate::app::Net;

/// the settings dialog
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.settings_open.clone();
    let was = open.get() != 0;
    if ui.dialog_begin(ID_SETTINGS_DLG, t("Einstellungen"), &open) {
        ui.next_size(480, 440);
        ui.scroll_begin(ID_SETTINGS_SCROLL, SC_V | pad(2));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | pad(4), 0);
        let changed = ui.settings_page(ID_SETTINGS_PAGE, &mut n.prefs.s);
        // what the program asked and may keep: the Akademie's connection, with its way back
        ui.spacer(12);
        ui.text(TS_SUBTITLE, t("Berechtigungen"));
        ui.spacer(4);
        ui.next_size(-1, 0);
        ui.permissions_page(ID_AI_PERM);
        ui.box_end();
        ui.scroll_end();
        if changed {
            take(n, ui);
        }
        ui.dialog_buttons(Some(t("Schließen")), None, None);
    }
    ui.dialog_end();
    if was && open.get() == 0 {
        let recent = n.file.recent.clone();
        n.prefs.save(&recent);
    }
}

// a value of the page changed: the window follows at once
fn take(n: &mut Net, ui: &mut Ui) {
    let (lang, motion, tempo, step) = (n.prefs.lang, n.prefs.motion, n.prefs.tempo, n.prefs.grid_step);
    n.prefs.pull();
    // the grid step: the open project takes it (saved with it, like the view)
    if n.prefs.grid_step != step {
        n.doc.proj.layout.grid = n.prefs.grid_step;
    }
    if n.prefs.lang != lang || n.prefs.motion != motion {
        n.prefs.apply(ui);
    }
    if n.prefs.tempo != tempo {
        ui.cmd_run(CMD_SIM_TEMPO0 + n.prefs.tempo as u32, 0);
    }
    ui.repass();
}

/// a row of the overview: the keys, what they do
struct Row {
    keys: String,
    what: &'static str,
}

// the commands of a menu that have an accelerator, in the menu's order
fn cmd_rows(ui: &Ui, ids: &[u32]) -> Vec<Row> {
    let mut v = Vec::new();
    for &id in ids {
        let keys = ui.accel_text(id);
        if keys.is_empty() {
            continue;
        }
        let label = ui.desc.cmds.iter().find(|c| c.id == id).map_or("", |c| c.label);
        v.push(Row { keys, what: label });
    }
    v
}

// keys that are no commands: (keys, what) in German, both translated
fn key_rows(list: &[(&'static str, &'static str)]) -> Vec<Row> {
    list.iter().map(|&(k, w)| Row { keys: String::from(t(k)), what: t(w) }).collect()
}

// i18n
const CANVAS_KEYS: [(&str, &str); 10] = [
    ("Tab / Umschalt+Tab", "Geräte und Notizen, dann die Kabel nacheinander wählen"),
    ("Pfeiltasten", "die Auswahl um einen Rasterschritt verschieben (Umschalt: zehn, Alt: einen Punkt); ohne Auswahl die Ansicht"),
    ("Eingabe", "Konsole des Geräts öffnen; Kabel: seine Eigenschaften; mit Verbinden: Ende A, dann B"),
    ("Umschalt+Doppelklick", "Gerätefenster öffnen"),
    ("Doppelklick auf freie Fläche", "Beschriftung dort schreiben (Eingabe übernimmt, Umschalt+Eingabe: neue Zeile, Esc verwirft)"),
    ("Strg+Plus / Strg+Minus", "vergrößern / verkleinern"),
    ("Leertaste + Ziehen", "die Ansicht verschieben"),
    ("Umschalt+F10 / Menütaste", "Kontextmenü des gewählten Geräts oder Kabels"),
    ("Esc", "Geste abbrechen, Auswahl aufheben, Werkzeug beenden"),
    ("Entf", "Auswahl löschen"),
];
// i18n
const CONSOLE_KEYS: [(&str, &str); 6] = [
    ("Tab", "Befehl oder Argument ergänzen"),
    ("?", "Hilfe zum Befehl (IOS-artige Konsolen)"),
    ("Pfeil auf / ab", "frühere Befehle"),
    ("Strg+C", "laufenden Befehl abbrechen"),
    ("Strg+Umschalt+6", "ping oder traceroute abbrechen (IOS-artig)"),
    ("Bild auf / Bild ab", "in der Ausgabe blättern"),
];
// i18n
const WINDOW_KEYS: [(&str, &str); 4] = [
    ("Umschalt+F6", "zum vorigen Bereich (F6 spielt die Simulation ab)"),
    ("F8", "Teiler zwischen den Bereichen mit den Pfeiltasten verschieben"),
    ("Strg+F6", "zwischen den Fenstern wechseln (Konsolen, Gerätefenster)"),
    ("Alt", "die Menüleiste"),
];
// i18n
const SCRIPT_KEYS: [(&str, &str); 19] = [
    ("Strg+S", "die Datei speichern (ein Rückgängig-Schritt des Projekts)"),
    ("Strg+Umschalt+S", "alle geänderten Dateien speichern"),
    ("Strg+W", "den Tab schließen (seine Änderungen bleiben)"),
    ("Strg+Bild ab / Bild auf", "zum nächsten / vorigen Tab"),
    ("Strg+Umschalt+F / H", "in Dateien suchen / ersetzen"),
    ("F5 / Strg+F5", "als Dienst ausführen (angehalten: fortsetzen) / als Prozess im Terminal"),
    ("Umschalt+F5 / Strg+Umschalt+F5", "stoppen / neu starten"),
    ("F9 / Strg+Umschalt+F9", "Haltepunkt an / aus, einer mit Bedingung"),
    ("F10 / F7 / Umschalt+F7", "Schritt über / hinein / heraus"),
    ("Strg+F9", "anhalten"),
    ("Strg+Umschalt+T", "die Tests der Datei ausführen"),
    ("F12 / F2 / Umschalt+Alt+F", "zur Definition / Name umbenennen / formatieren"),
    ("F8 / Umschalt+F8", "zur nächsten / vorigen Meldung (Fehler, Warnung, Fehler im Lauf)"),
    ("Strg+Z / Strg+Y", "im Editor rückgängig / wiederholen (außerhalb des Editors: das Projekt)"),
    ("Strg+F / Strg+H", "in der Datei suchen / ersetzen"),
    ("Strg+B / Strg+J", "Seitenleiste / untere Leiste ein und aus"),
    ("Strg+/", "Zeilen aus- und wieder einkommentieren"),
    ("Tab / Umschalt+Tab", "die gewählten Zeilen ein- und ausrücken"),
    ("Alt+Pfeil auf / ab", "die Zeilen verschieben"),
];
// i18n
const DESK_KEYS: [(&str, &str); 4] = [
    ("Alt+F6 / Alt+Umschalt+F6", "zum nächsten / vorigen Fenster des Desktops"),
    ("Alt+F10 / Alt+F9", "das aktive Fenster maximieren und zurück / minimieren"),
    ("Alt+Minus", "das Menü des aktiven Fensters"),
    ("Doppelklick / Eingabe", "das Programm eines Symbols öffnen"),
];

/// the overview of the shortcuts
pub(crate) fn shortcuts(n: &mut Net, ui: &mut Ui) {
    let open = n.keys_open.clone();
    if ui.dialog_begin(ID_KEYS_DLG, t("Tastenkürzel"), &open) {
        let sections: [(&'static str, Vec<Row>); 11] = [
            (t("Datei und Bearbeiten"), cmd_rows(ui, &[CMD_NEW, CMD_OPEN, CMD_SAVE, CMD_SAVE_AS, CMD_QUIT, CMD_UNDO, CMD_REDO, CMD_DELETE, CMD_SELECT_ALL, CMD_RENAME, CMD_GROUP, CMD_UNGROUP])),
            (t("Werkzeuge"), cmd_rows(ui, &[CMD_TOOL_SELECT, CMD_TOOL_HAND, CMD_TOOL_CONNECT, CMD_TOOL_NOTE, CMD_LABEL, CMD_TOOL_PDU, CMD_SETTINGS])),
            (t("Ansicht"), cmd_rows(ui, &[CMD_ZOOM_IN, CMD_ZOOM_OUT, CMD_FIT, CMD_VIEW_LOGICAL, CMD_VIEW_PHYS, CMD_PHYS_UP, CMD_PHYS_BACK])),
            (t("Bereiche"), cmd_rows(ui, &[CMD_FOCUS_CANVAS, CMD_PANEL0, CMD_PANEL0 + 1, CMD_PANEL0 + 2, CMD_PANEL0 + 3, CMD_PANEL0 + 4, CMD_PANEL0 + 5, CMD_PANEL0 + 6, CMD_PANEL0 + 7, CMD_METRICS, CMD_EXPERIMENTS])),
            (t("Simulation"), cmd_rows(ui, &[CMD_SIM_MODE, CMD_SIM_REALTIME, CMD_SIM_PLAY, CMD_SIM_STEP, CMD_SIM_BACK])),
            (t("Topologie"), key_rows(&CANVAS_KEYS)),
            (t("Konsolen"), key_rows(&CONSOLE_KEYS)),
            (t("Fenster"), {
                let mut v = cmd_rows(ui, &[CMD_SHORTCUTS]);
                v.extend(key_rows(&WINDOW_KEYS));
                v
            }),
            (t("Aufgabe"), cmd_rows(ui, &[CMD_TASK_PANEL, CMD_TASK_CHECK])),
            (t("IDE"), key_rows(&SCRIPT_KEYS)),
            (t("Desktop"), {
                let mut v = cmd_rows(ui, &[CMD_DESK_FULL]);
                v.extend(key_rows(&DESK_KEYS));
                v
            }),
        ];
        ui.next_size(500, 460);
        ui.scroll_begin(ID_KEYS_SCROLL, SC_V | pad(2));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | pad(4), 10);
        for (title, rows) in sections.iter() {
            ui.text(TS_BODY_STRONG, title);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 4);
            for r in rows {
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_H, 12);
                ui.next_size(170, 0);
                ui.text(TS_BODY_STRONG | TX_WRAP, &r.keys);
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_WRAP, &strip_amp(r.what));
                ui.box_end();
            }
            ui.box_end();
        }
        ui.box_end();
        ui.scroll_end();
        ui.dialog_buttons(Some(t("Schließen")), None, None);
    }
    ui.dialog_end();
}

// a menu label without its mnemonic and the dots of a dialog ("&Speichern unter …")
fn strip_amp(s: &str) -> String {
    let mut out = String::new();
    let mut amp = false;
    for c in s.chars() {
        if c == '&' && !amp {
            amp = true;
            continue;
        }
        amp = false;
        out.push(c);
    }
    String::from(out.trim_end_matches(" …").trim_end_matches('…').trim_end())
}
