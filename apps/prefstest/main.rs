// prefstest -- typed settings and the page built from their schema (base::settings,
// ui.settings_page), the placement of the windows kept between runs (state_save /
// state_load, AF_SAVE_STATE) and widget ids from names (ui.id, id_scope, wid), for
// tests/page.uis, tests/state.uis and tests/ids.uis
//
//   build/win/prefstest.exe --script apps/prefstest/tests/page.uis
//   build/win/prefstest.exe          (a window: its placement and the palette's go to
//                                     config://windows.ini; jfgui_STATE=FILE elsewhere)
//
// The settings live in mem://prefstest/prefs.ini ("Speichern" / "Neu laden"), the window
// state in mem://prefstest/windows.ini ("Zustand speichern" / "Zustand laden"): tests never
// write real files.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use base::settings::rename;
use base::{Ini, Key, Schema, Settings};
use ui::*;
use ui::i18n::{t, tf, Catalog};

const DESC: AppDesc =
    AppDesc { title: "JFLib Einstellungen", flags: AF_RESIZABLE | AF_SAVE_STATE, w: 760, h: 1000, min_w: 400, min_h: 300, ..AppDesc::NONE }; // i18n

const PREFS: &str = "mem://prefstest/prefs.ini";
const STATE: &str = "mem://prefstest/windows.ini";

const DARK: Key<bool> = Key::bool("view.dark", false).label("Dunkles Design");
const ZOOM: Key<i64> = Key::int("view.zoom", 100, 10, 800).label("Zoom (%)").step(10).help("Die Größe beim Öffnen eines Bildes.");
const SCALE: Key<f64> = Key::num("view.scale", 1.5, 0.5, 4.0, 2).label("Linienstärke");
const UNIT: Key<usize> =
    Key::choice("view.unit", 0, &[("px", "Pixel"), ("cm", "Zentimeter"), ("in", "Zoll")]).label("Einheit");
const ACCENT: Key<u32> = Key::color("view.accent", 0x0078D4).label("Akzentfarbe");
const NAME: Key<String> = Key::text("user.name", "Gast", 16).label("Name");
const THREADS: Key<i64> = Key::int("system.threads", 4, 1, 64).label("Rechenkerne").restart();
const LAST: Key<String> = Key::text("system.last", "", 260).hidden(); // state the app keeps

static SCHEMA: Schema = Schema::new(2, &[DARK.d, ZOOM.d, SCALE.d, UNIT.d, ACCENT.d, NAME.d, THREADS.d, LAST.d])
    .sections(&[("view", "Ansicht"), ("user", "Benutzer")])
    .migrate(migrate);

// version 1 kept the zoom as "view.size"
fn migrate(from: u32, ini: &mut Ini) {
    if from < 2 {
        rename(ini, "view.size", "view.zoom");
    }
}

const PALETTE: u32 = wid("palette");
const ITEMS: [&str; 3] = ["Erster", "Zweiter", "Dritter"]; // i18n
const UNITS: [&str; 3] = ["Pixel", "Zentimeter", "Zoll"]; // i18n

struct Prefs {
    s: Settings,
    palette: Shared<i32>,
    palette_rect: Shared<WinRect>,
    deleted: [u32; 3],
    file: String,
    open_scope: i32,
    twins: i32,
    clash: i32,
}

impl Prefs {
    fn new() -> Prefs {
        // a file of version 1: migrated on open
        plat::write_file(PREFS, b"version=1\n[view]\nsize=150\n");
        Prefs {
            s: Settings::open(PREFS, &SCHEMA),
            palette: Shared::new(0),
            palette_rect: Shared::new(WinRect::new(0, 0, 220, 160, 0)),
            deleted: [0; 3],
            file: String::new(),
            open_scope: 0,
            twins: 0,
            clash: 0,
        }
    }
}

fn yn(b: bool) -> &'static str {
    if b { t("ja") } else { t("nein") }
}

// a file in one line: "a | b | c"
fn one_line(path: &str) -> String {
    let data = plat::read_file(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&data);
    let parts: alloc::vec::Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    parts.join(" | ")
}

impl App for Prefs {
    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, t("Einstellungen"));
        // the typed values
        let s = &self.s;
        let unit = t(UNITS[s.get(&UNIT)]);
        ui.text(TS_BODY, &tf("Zoom: {}  Dunkel: {}  Einheit: {}  Farbe: {}  Name: {}  Linie: {}  Kerne: {}",
            &[&format!("{}", s.get(&ZOOM)), yn(s.get(&DARK)), unit, &format!("#{:06X}", s.get(&ACCENT)), s.text(&NAME),
              &format!("{:.2}", s.get(&SCALE)), &format!("{}", s.get(&THREADS))]));
        ui.text(TS_BODY, &tf("Version der Datei: {}  Ungespeichert: {}", &[&format!("{}", s.file_version()), yn(s.dirty())]));
        let (cw, ch) = ui.client_size();
        ui.text(TS_BODY, &tf("Fenster: {} × {}", &[&format!("{}", cw), &format!("{}", ch)]));
        ui.text(TS_CAPTION, &tf("Datei: {}", &[&self.file]));
        // ids: the constants equal the names in their scopes
        let root = ui.id("save") == wid("save");
        ui.id_push_str("layers");
        let scoped = ui.id("add") == wid_path(&["layers", "add"]);
        ui.id_pop();
        ui.text(TS_BODY, &tf("IDs: {}", &[if root && scoped { t("gleich") } else { t("verschieden") }]));
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        if ui.button(ui.id("save"), t("Speichern")) & EV_ACT != 0 {
            self.s.save();
            self.file = one_line(PREFS);
        }
        if ui.button(ui.id("reload"), t("Neu laden")) & EV_ACT != 0 {
            self.s = Settings::open(PREFS, &SCHEMA);
        }
        if ui.button(ui.id("state.save"), t("Zustand speichern")) & EV_ACT != 0 {
            let mut ini = Ini::load(STATE);
            ui.state_save(&mut ini);
            ini.save(STATE);
            self.file = one_line(STATE);
        }
        if ui.button(ui.id("state.load"), t("Zustand laden")) & EV_ACT != 0 {
            ui.state_load(&Ini::load(STATE));
        }
        let mut open = self.palette.get();
        if ui.toggle_button(ui.id("palette.toggle"), t("Palette"), &mut open) & EV_ACT != 0 {
            self.palette.set(open);
        }
        ui.box_end();
        // rows: the same names in one scope per row
        ui.box_begin(0, BOX_H, 8);
        for (i, name) in ITEMS.iter().enumerate() {
            let label = tf("Löschen {}", &[&format!("{}", i + 1)]);
            let ev = ui.id_scope(i as u32, |ui| {
                ui.text(TS_BODY, t(*name));
                ui.button(ui.id("del"), &label)
            });
            if ev & EV_ACT != 0 {
                self.deleted[i] += 1;
            }
        }
        ui.box_end();
        ui.text(TS_BODY, &tf("Gelöscht: {} {} {}", &[&format!("{}", self.deleted[0]), &format!("{}", self.deleted[1]), &format!("{}", self.deleted[2])]));
        // mistakes the library reports
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        ui.toggle_button(ui.id("err.scope"), t("Offener Bereich"), &mut self.open_scope);
        ui.toggle_button(ui.id("err.twins"), t("Gleiche Namen"), &mut self.twins);
        ui.toggle_button(ui.id("err.clash"), t("Kollision"), &mut self.clash);
        if self.twins != 0 {
            ui.button(ui.id("twin"), t("Zwilling A"));
            ui.button(ui.id("twin"), t("Zwilling B"));
        }
        if self.clash != 0 {
            ui.button(ui.id("k11513"), t("Name k11513"));
            ui.button(ui.id("k27407"), t("Name k27407"));
        }
        ui.box_end();
        if self.open_scope != 0 {
            ui.id_push(7); // never popped: a UIWARN at the end of the frame
        }
        // the page built from the schema
        if ui.settings_page(wid("page"), &mut self.s) {
            self.s.set(&LAST, String::from("geändert"));
        }
        ui.box_end();
        // a palette (tool window): open and placement kept by state_save
        if ui.window_begin(PALETTE, t("Werkzeuge"), Some(&self.palette), WF_TOOL | WF_RESIZABLE, Some(&self.palette_rect)) {
            ui.box_begin(0, BOX_V | pad(12), 6);
            ui.text(TS_BODY, t("Pinsel"));
            ui.text(TS_BODY, t("Radierer"));
            ui.box_end();
        }
        ui.window_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id == UIC_EXIT {
            self.s.save();
            return true;
        }
        false
    }
}

ui::main!({
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")).with_library_languages());
    ui::launch(&DESC.translated(), Prefs::new(), &style::STYLE)
});
