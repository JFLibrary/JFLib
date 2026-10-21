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

const DESC: AppDesc =
    AppDesc { title: "jfgui Einstellungen", flags: AF_RESIZABLE | AF_SAVE_STATE, w: 760, h: 1000, min_w: 400, min_h: 300, ..AppDesc::NONE };

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
const ITEMS: [&str; 3] = ["Erster", "Zweiter", "Dritter"];

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
    if b { "ja" } else { "nein" }
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
        ui.text(TS_SUBTITLE, "Einstellungen");
        // the typed values
        let s = &self.s;
        let unit = ["Pixel", "Zentimeter", "Zoll"][s.get(&UNIT)];
        ui.text(TS_BODY, &format!("Zoom: {}  Dunkel: {}  Einheit: {}  Farbe: #{:06X}  Name: {}  Linie: {:.2}  Kerne: {}",
            s.get(&ZOOM), yn(s.get(&DARK)), unit, s.get(&ACCENT), s.text(&NAME), s.get(&SCALE), s.get(&THREADS)));
        ui.text(TS_BODY, &format!("Version der Datei: {}  Ungespeichert: {}", s.file_version(), yn(s.dirty())));
        let (cw, ch) = ui.client_size();
        ui.text(TS_BODY, &format!("Fenster: {} × {}", cw, ch));
        ui.text(TS_CAPTION, &format!("Datei: {}", self.file));
        // ids: the constants equal the names in their scopes
        let root = ui.id("save") == wid("save");
        ui.id_push_str("layers");
        let scoped = ui.id("add") == wid_path(&["layers", "add"]);
        ui.id_pop();
        ui.text(TS_BODY, &format!("IDs: {}", if root && scoped { "gleich" } else { "verschieden" }));
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        if ui.button(ui.id("save"), "Speichern") & EV_ACT != 0 {
            self.s.save();
            self.file = one_line(PREFS);
        }
        if ui.button(ui.id("reload"), "Neu laden") & EV_ACT != 0 {
            self.s = Settings::open(PREFS, &SCHEMA);
        }
        if ui.button(ui.id("state.save"), "Zustand speichern") & EV_ACT != 0 {
            let mut ini = Ini::load(STATE);
            ui.state_save(&mut ini);
            ini.save(STATE);
            self.file = one_line(STATE);
        }
        if ui.button(ui.id("state.load"), "Zustand laden") & EV_ACT != 0 {
            ui.state_load(&Ini::load(STATE));
        }
        let mut open = self.palette.get();
        if ui.toggle_button(ui.id("palette.toggle"), "Palette", &mut open) & EV_ACT != 0 {
            self.palette.set(open);
        }
        ui.box_end();
        // rows: the same names in one scope per row
        ui.box_begin(0, BOX_H, 8);
        for (i, name) in ITEMS.iter().enumerate() {
            let label = format!("Löschen {}", i + 1);
            let ev = ui.id_scope(i as u32, |ui| {
                ui.text(TS_BODY, name);
                ui.button(ui.id("del"), &label)
            });
            if ev & EV_ACT != 0 {
                self.deleted[i] += 1;
            }
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Gelöscht: {} {} {}", self.deleted[0], self.deleted[1], self.deleted[2]));
        // mistakes the library reports
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        ui.toggle_button(ui.id("err.scope"), "Offener Bereich", &mut self.open_scope);
        ui.toggle_button(ui.id("err.twins"), "Gleiche Namen", &mut self.twins);
        ui.toggle_button(ui.id("err.clash"), "Kollision", &mut self.clash);
        if self.twins != 0 {
            ui.button(ui.id("twin"), "Zwilling A");
            ui.button(ui.id("twin"), "Zwilling B");
        }
        if self.clash != 0 {
            ui.button(ui.id("k11513"), "Name k11513");
            ui.button(ui.id("k27407"), "Name k27407");
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
        if ui.window_begin(PALETTE, "Werkzeuge", Some(&self.palette), WF_TOOL | WF_RESIZABLE, Some(&self.palette_rect)) {
            ui.box_begin(0, BOX_V | pad(12), 6);
            ui.text(TS_BODY, "Pinsel");
            ui.text(TS_BODY, "Radierer");
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

ui::main!(ui::launch(&DESC, Prefs::new(), &style::STYLE));
