// options -- the settings of JFDiagrammer (File › Optionen, FEATURES.md 3.1 UI-13 .. UI-18,
// P1 part) through the library's typed settings (base::settings), and their pages
//
//   SCHEMA                    the keys, in the sections general, save, language, advanced
//   open(ui) -> Settings      config://settings.ini in a window; in headless runs (scripts,
//                             shots) the settings live in memory unless --config PATH names a
//                             file, so a script never touches the user's settings
//   page(app, ui, section)    one options page: the library's settings_section; a change is
//                             saved at once and takes effect at once (language, region,
//                             theme)
//   keys_page(app, ui)        the shortcuts of the current language (read only; own ones go
//                             into [keys] of the settings file, UI-19 comes later)
//
// What the options do today: the language and the region switch at once (LG-02, LG-07), the
// theme follows "Design"; the rest is stored and waits for its stage (start screen JD13,
// AutoRecover JD5, live dynamics and units JD7, undo depth JD2's history in the app).

use alloc::string::String;
use base::settings::{Key, Schema, Settings};
use ui::i18n::t;
use ui::*;

use crate::{commands, Diagrammer};

// the labels, help texts and choice labels are rows of locales/ui.tsv (the settings page
// translates them; i18n_check does not see them, so they are kept there by hand)
pub(crate) const THEME: Key<usize> = Key::choice("general.theme", 0, &[("system", "Systemeinstellung"), ("light", "Hell"), ("dark", "Dunkel")])
    .label("Erscheinungsbild");
pub(crate) const USER: Key<String> = Key::text("general.user", "", 64)
    .label("Benutzername")
    .help("Steht an Ihren Kommentaren.");
pub(crate) const INITIALS: Key<String> = Key::text("general.initials", "", 8)
    .label("Initialen");
pub(crate) const START: Key<bool> = Key::bool("general.start_screen", true)
    .label("Startbildschirm beim Programmstart anzeigen");
pub(crate) const LIVE: Key<bool> = Key::bool("general.live_preview", true)
    .label("Livevorschau aktivieren");
pub(crate) const TIPS: Key<usize> = Key::choice("general.screentips", 0, &[("full", "Mit Beschreibung"), ("short", "Ohne Beschreibung"), ("none", "Keine QuickInfos")])
    .label("QuickInfos");
pub(crate) const FORMAT: Key<usize> = Key::choice("save.format", 0, &[("vsdx", "Visio-Zeichnung (.vsdx)")])
    .label("Dateien in diesem Format speichern");
pub(crate) const AUTOREC: Key<bool> = Key::bool("save.autorecover", true)
    .label("AutoWiederherstellen-Informationen speichern");
pub(crate) const AUTOREC_MIN: Key<i64> = Key::int("save.autorecover_minutes", 10, 1, 120)
    .label("Intervall (Minuten)");
pub(crate) const DRAWINGS: Key<String> = Key::text("save.drawings", "", 1024)
    .label("Standardspeicherort für Zeichnungen");
pub(crate) const TEMPLATES: Key<String> = Key::text("save.templates", "", 1024)
    .label("Speicherort für persönliche Vorlagen");
pub(crate) const UI_LANG: Key<usize> = Key::choice("language.ui", 0, &[("system", "Wie das System"), ("de", "Deutsch"), ("en", "English"), ("es", "Español"), ("it", "Italiano"), ("fr", "Français")])
    .label("Sprache der Oberfläche")
    .help("Gilt sofort, ohne Neustart.");
pub(crate) const REGION: Key<usize> = Key::choice("language.region", 0, &[("", "Wie die Sprache"), ("de-DE", "Deutschland"), ("en-US", "Vereinigte Staaten"), ("en-GB", "Vereinigtes Königreich"), ("es-ES", "Spanien"), ("it-IT", "Italien"), ("fr-FR", "Frankreich")])
    .label("Region für Einheiten und Formate");
pub(crate) const LIVE_DYN: Key<bool> = Key::bool("advanced.live_dynamics", true)
    .label("Livedynamik aktivieren");
pub(crate) const WHEEL_ZOOM: Key<bool> = Key::bool("advanced.wheel_zoom", false)
    .label("Mit dem Rad zoomen");
pub(crate) const CENTER_ZOOM: Key<bool> = Key::bool("advanced.center_zoom", true)
    .label("Auswahl beim Zoomen zentrieren");
pub(crate) const CONNECT_ARROWS: Key<bool> = Key::bool("advanced.connect_arrows", true)
    .label("Verbindungspfeile aktivieren");
pub(crate) const SPLIT_CONN: Key<bool> = Key::bool("advanced.split_connectors", true)
    .label("Verbinder teilen");
pub(crate) const DELETE_CONN: Key<bool> = Key::bool("advanced.delete_connectors", true)
    .label("Verbinder mit Shapes löschen");
pub(crate) const ENTER_ENDS: Key<bool> = Key::bool("advanced.enter_ends_text", false)
    .label("Eingabetaste beendet die Textbearbeitung");
pub(crate) const UNDO_LEVELS: Key<i64> = Key::int("advanced.undo_levels", 20, 1, 99)
    .label("Anzahl der Rückgängig-Schritte");
pub(crate) const UNITS: Key<usize> = Key::choice("advanced.units", 0, &[("auto", "Wie die Region"), ("mm", "Millimeter"), ("cm", "Zentimeter"), ("in", "Zoll"), ("pt", "Punkt")])
    .label("Maßeinheit der Seite");
pub(crate) const DEV: Key<bool> = Key::bool("advanced.developer", false)
    .label("Im Entwicklermodus ausführen");

static SCHEMA: Schema = Schema::new(1, &[
    THEME.d, USER.d, INITIALS.d, START.d, LIVE.d, TIPS.d,
    FORMAT.d, AUTOREC.d, AUTOREC_MIN.d, DRAWINGS.d, TEMPLATES.d,
    UI_LANG.d, REGION.d,
    LIVE_DYN.d, WHEEL_ZOOM.d, CENTER_ZOOM.d, CONNECT_ARROWS.d, SPLIT_CONN.d, DELETE_CONN.d, ENTER_ENDS.d, UNDO_LEVELS.d, UNITS.d, DEV.d,
])
.sections(SECTIONS);

// the sections' titles: PAGES below
static SECTIONS: &[(&str, &str)] = &[("general", "Allgemein"), ("save", "Speichern"), ("language", "Sprache"), ("advanced", "Erweitert")];

/// the options' pages, in the order of the list at their side (the last is the shortcuts)
pub(crate) const PAGES: [&str; 5] = ["Allgemein", "Speichern", "Sprache", "Erweitert", "Tastenkürzel"]; // i18n
const KEYS: [&str; 4] = ["general", "save", "language", "advanced"];

/// the schema (Settings::new before the settings are opened in init)
pub(crate) fn schema() -> &'static Schema {
    &SCHEMA
}

/// the settings: the file in a window, memory in headless runs (unless --config PATH)
pub(crate) fn open(ui: &Ui) -> Settings {
    match plat::arg_value("--config") {
        Some(p) if !p.is_empty() => Settings::open(&p, &SCHEMA),
        _ if ui.headless() => Settings::new(&SCHEMA),
        _ => Settings::open("config://settings.ini", &SCHEMA),
    }
}

/// the theme of the option (0 system, 1 light, 2 dark) -> ui.theme_set's mode
pub(crate) fn theme_mode(choice: usize) -> u32 {
    match choice {
        1 => 0,
        2 => 1,
        _ => 2,
    }
}

/// one options page (`page` an index of PAGES)
pub(crate) fn page(app: &mut Diagrammer, ui: &mut Ui, page: usize) {
    if page >= KEYS.len() {
        keys_page(app, ui);
        return;
    }
    let before = (app.opts.get(&UI_LANG), app.opts.get(&REGION), app.opts.get(&THEME));
    ui.next_size(560, 0);
    if ui.settings_section(0x7100, &mut app.opts, KEYS[page]) {
        app.opts.save();
        let now = (app.opts.get(&UI_LANG), app.opts.get(&REGION), app.opts.get(&THEME));
        if now.0 != before.0 || now.1 != before.1 {
            app.apply_language(ui);
        }
        if now.2 != before.2 {
            ui.theme_set(theme_mode(now.2));
        }
    }
}

// the shortcuts of the current language: command and keys, two columns
fn keys_page(_app: &mut Diagrammer, ui: &mut Ui) {
    ui.text(TS_BODY_STRONG, t("Tastenkürzel"));
    ui.next_size(560, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Die Tastenkürzel folgen der Office-Belegung der Oberflächensprache. Eigene Kürzel stehen im Abschnitt [keys] der Einstellungsdatei."));
    ui.spacer(8);
    ui.next_size(560, 0);
    ui.box_begin(0x7200, BOX_V | BOX_CARD | pad(12), 2);
    let cmds: &'static [Command] = ui.desc.cmds;
    let mut seen: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
    for c in cmds.iter().take_while(|c| c.id != 0) {
        if c.key == 0 || seen.contains(&c.id) {
            continue;
        }
        seen.push(c.id);
        let keys: alloc::vec::Vec<String> = cmds.iter().take_while(|k| k.id != 0).filter(|k| k.id == c.id && k.key != 0).map(|k| ui.key_text(k.key as u32, k.mods as u32)).collect();
        ui.box_begin(0, BOX_H, 12);
        ui.next_size(-1, 0);
        ui.text(TS_BODY, commands::label(c.id));
        ui.next_size(200, 0);
        ui.text(TS_BODY | TX_SECONDARY, &keys.join(", "));
        ui.box_end();
    }
    ui.box_end();
}
