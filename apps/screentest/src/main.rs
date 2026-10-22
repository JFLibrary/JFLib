// screentest -- the screen services of the library for the app (ui/screen.rs; plat::screen,
// plat::hotkey): the monitors and their changes, a global shortcut, a colour from anywhere on the
// screen (the colour picker's pipette and ui.pick_color), a picture of the screen
//
//   python build_rs.py run screentest
//   python build_rs.py test screentest          tests/*.uis: the UI headless (shortcuts only
//                                               recorded, `system hotkey ID` presses one, the
//                                               pick answered by `system pick RRGGBB|cancel`)
//   python build_rs.py app screentest --target web, then
//   node apps/screentest/tests/screen_web.mjs build [--engines=chromium,firefox]
//
// The page shows each monitor (name, rectangle, work area, dpi, refresh rate, primary) and how
// often the list changed (UIC_SETTINGS with plat::SET_MONITORS), registers the chord of its
// text box as global shortcut 1 (UIC_HOTKEY counts the presses, UIC_HOTKEY_LOST shows why the
// desktop dropped it), has a colour picker with CP_PIPETTE and a button for ui.pick_color
// (UIC_PICK), and takes a picture of the screen (UIC_SCREENSHOT) shown scaled down.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;

use gfx::Surface;
use ui::*;

const ID_PAGE: u32 = 100;
const ID_CHORD: u32 = 101;
const ID_REG: u32 = 102;
const ID_PICKER: u32 = 103;
const ID_PICK: u32 = 104;
const ID_SHOT: u32 = 105;
const ID_IMAGE: u32 = 106;

const HOTKEY: u32 = 1;
const CMD_QUIT: u32 = 1;

static CMDS: [Command; 1] = [Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0)];
static M_FILE: [Mi; 1] = [Mi::Cmd(CMD_QUIT)];
static MENUBAR: [Mi; 1] = [Mi::Menu("&Datei", &M_FILE)];

const DESC: AppDesc = AppDesc {
    title: "Bildschirm-Test",
    flags: AF_RESIZABLE,
    w: 720,
    h: 820,
    min_w: 420,
    min_h: 360,
    cmds: &CMDS,
    menubar: &MENUBAR,
    ..AppDesc::NONE
};

struct Screentest {
    monitors: alloc::vec::Vec<Monitor>,
    changes: u32,
    chord: String,
    registered: bool,
    hotkey_note: String,
    presses: u32,
    color: u32,
    pick_note: String,
    shot: Option<Rc<Surface>>,
    shot_ver: u32,
    shot_note: String,
    started: bool,
}

impl Default for Screentest {
    fn default() -> Self {
        Screentest {
            monitors: alloc::vec::Vec::new(),
            changes: 0,
            chord: String::from("Ctrl+Alt+K"),
            registered: false,
            hotkey_note: String::new(),
            presses: 0,
            color: 0xFF20_6090,
            pick_note: String::new(),
            shot: None,
            shot_ver: 0,
            shot_note: String::new(),
            started: false,
        }
    }
}

fn why_hotkey(e: &HotkeyError) -> &'static str {
    match e {
        HotkeyError::Unavailable => "hier nicht verfügbar",
        HotkeyError::Denied => "nicht erlaubt (input.global fehlt)",
        HotkeyError::Invalid => "ungültiges Kürzel",
        HotkeyError::Conflict => "schon vergeben",
        HotkeyError::Dismissed => "vom Desktop nicht vergeben",
        HotkeyError::Failed(_) => "fehlgeschlagen",
    }
}

fn why_screen(e: Option<ScreenError>) -> String {
    match e {
        Some(ScreenError::Unavailable) => String::from("hier nicht verfügbar"),
        Some(ScreenError::Denied) => String::from("nicht erlaubt"),
        Some(ScreenError::Cancelled) => String::from("abgebrochen"),
        Some(ScreenError::Busy) => String::from("läuft schon"),
        Some(ScreenError::Failed(s)) => format!("fehlgeschlagen: {s}"),
        None => String::from("abgebrochen"),
    }
}

fn monitor_text(k: usize, m: &Monitor) -> String {
    let hz = if m.refresh_mhz > 0 { format!(" · {}.{:02} Hz", m.refresh_mhz / 1000, m.refresh_mhz % 1000 / 10) } else { String::new() };
    format!(
        "{}: {} – {} × {} bei {}, {} · Arbeitsbereich {} × {} bei {}, {} · {} dpi{}{}",
        k + 1,
        if m.name.is_empty() { "?" } else { &m.name },
        m.rect[2],
        m.rect[3],
        m.rect[0],
        m.rect[1],
        m.work[2],
        m.work[3],
        m.work[0],
        m.work[1],
        m.dpi,
        hz,
        if m.primary { " · primär" } else { "" }
    )
}

impl Screentest {
    /// jfgui_SELFTEST=screen (Android: a string extra of the start intent of a debug build): the
    /// monitors written to data://screen-selftest.txt at the start and after every change, one
    /// line each "monitor ID X Y W H WX WY WW WH DPI MHZ PRIMARY NAME", then "changes N"
    /// (tests/android_screen.py)
    fn selftest_write(&self) {
        if plat::env("jfgui_SELFTEST").as_deref() != Some("screen") {
            return;
        }
        let mut s = String::new();
        for m in &self.monitors {
            s.push_str(&format!(
                "monitor {} {} {} {} {} {} {} {} {} {} {} {} {}\n",
                m.id, m.rect[0], m.rect[1], m.rect[2], m.rect[3], m.work[0], m.work[1], m.work[2], m.work[3], m.dpi, m.refresh_mhz, m.primary as u32, m.name
            ));
        }
        s.push_str(&format!("changes {}\n", self.changes));
        plat::write_file("data://screen-selftest.txt", s.as_bytes());
    }

    fn toggle_hotkey(&mut self, ui: &mut Ui) {
        if self.registered {
            ui.hotkey_unregister(HOTKEY);
            self.registered = false;
            self.hotkey_note = String::from("Entfernt.");
            return;
        }
        match ui.hotkey_register(HOTKEY, &self.chord.clone(), "Bildschirm-Test: das Kürzel zählen") {
            Ok(()) => {
                self.registered = true;
                self.presses = 0;
                let c = Chord::parse(&self.chord).map_or(self.chord.clone(), |c| c.text());
                self.hotkey_note = format!("Registriert: {c}");
            }
            Err(e) => self.hotkey_note = format!("Nicht registriert: {}", why_hotkey(&e)),
        }
    }
}

impl App for Screentest {
    fn frame(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            self.monitors = ui.monitors();
            self.selftest_write();
        }
        ui.menubar(None);
        ui.next_size(0, -1);
        ui.scroll_begin(ID_PAGE, SC_V);
        ui.box_begin(0, BOX_V | pad(20), 14);
        ui.heading(1, TS_TITLE, "Bildschirm-Test");

        // the monitors
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.heading(2, TS_SUBTITLE, "Monitore");
        for (k, m) in self.monitors.iter().enumerate() {
            ui.text(TS_BODY | TX_WRAP, &monitor_text(k, m));
        }
        if self.monitors.is_empty() {
            ui.text(TS_BODY, "Keine Monitore bekannt.");
        }
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Änderungen: {}", self.changes));
        ui.box_end();

        // a global shortcut
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.heading(2, TS_SUBTITLE, "Globales Tastenkürzel");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(200, 0);
        ui.textbox(ID_CHORD, Some("Strg+Alt+K"), &mut self.chord, 64);
        let label = if self.registered { "Entfernen" } else { "Registrieren" };
        if ui.button(ID_REG, label) & EV_ACT != 0 {
            self.toggle_hotkey(ui);
        }
        ui.box_end();
        if !ui.hotkeys_available() {
            ui.text(TS_BODY, "Globale Tastenkürzel: hier nicht verfügbar");
        }
        if !self.hotkey_note.is_empty() {
            ui.text(TS_BODY | TX_WRAP, &self.hotkey_note);
        }
        ui.text(TS_BODY, &format!("Gedrückt: {}", self.presses));
        ui.box_end();

        // a colour from the screen
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.heading(2, TS_SUBTITLE, "Farbe vom Bildschirm");
        ui.color_picker_ex(ID_PICKER, &mut self.color, CP_FIELDS | CP_PIPETTE);
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Gewählt: #{:06X}", self.color & 0xFF_FFFF));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.button(ID_PICK, "Farbe aufnehmen") & EV_ACT != 0 {
            self.pick_note = if ui.pick_color() { String::from("Klicken Sie irgendwo auf den Bildschirm …") } else { format!("Farbe aufnehmen: {}", why_screen(ui.screen_error())) };
        }
        if !self.pick_note.is_empty() {
            ui.text(TS_BODY, &self.pick_note);
        }
        ui.box_end();
        ui.box_end();

        // a picture of the screen
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.heading(2, TS_SUBTITLE, "Bildschirmfoto");
        if ui.button(ID_SHOT, "Bildschirm aufnehmen") & EV_ACT != 0 {
            self.shot_note = if ui.screenshot(None) { String::from("Wird aufgenommen …") } else { format!("Bildschirmfoto: {}", why_screen(ui.screen_error())) };
        }
        if !self.shot_note.is_empty() {
            ui.text(TS_BODY | TX_WRAP, &self.shot_note);
        }
        if let Some(sf) = &self.shot {
            let w = 320.min(sf.w);
            ui.next_size(w, (sf.h as i64 * w as i64 / sf.w.max(1) as i64) as i32);
            ui.image(ID_IMAGE, sf, self.shot_ver);
        }
        ui.box_end();
        ui.box_end();
        ui.scroll_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_QUIT => ui.quit(),
            UIC_SETTINGS if arg & plat::SET_MONITORS != 0 => {
                self.monitors = ui.monitors();
                self.changes += 1;
                self.selftest_write();
            }
            UIC_HOTKEY if arg == HOTKEY => self.presses += 1,
            UIC_HOTKEY_LOST if arg == HOTKEY => {
                self.registered = false;
                self.hotkey_note = format!("Vom Desktop entfernt: {}", ui.hotkey_error(HOTKEY).as_ref().map_or("?", why_hotkey));
            }
            UIC_PICK => {
                self.pick_note = match ui.picked_color() {
                    Some(c) => format!("Aufgenommen: #{c:06X}"),
                    None => format!("Farbe aufnehmen: {}", why_screen(ui.screen_error())),
                };
            }
            UIC_SCREENSHOT => match ui.screenshot_take() {
                Some(sf) => {
                    self.shot_note = format!("Aufgenommen: {} × {}", sf.w, sf.h);
                    self.shot = Some(Rc::new(sf));
                    self.shot_ver += 1;
                }
                None => self.shot_note = format!("Bildschirmfoto: {}", why_screen(ui.screen_error())),
            },
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, Screentest::default(), &style::STYLE));
