// docktest -- the docking framework (ui/dock.rs): six panels around a document, for
// tests/*.uis and by hand
//
//   build/win/docktest.exe --layout build/dock.txt  by hand: drag tabs, float and dock them
//                                                  (--layout: loaded at the start, Speichern
//                                                  writes it)
//   build/win/docktest.exe --script apps/docktest/tests/drag.uis
//
// Panels: Dateien and Gliederung (left, one group), Eigenschaften (right), Ausgabe and Suche
// (bottom, two groups), Verlauf (right, second group, starts hidden).  The tool row saves the
// layout into memory (Speichern), loads it again (Laden), resets it and shows / hides the
// Verlauf; Ctrl+1 .. Ctrl+6 focus a panel (DockState::focus).  The status line describes the
// layout: per side its groups ("[shown]+other"), "(zu)" for a collapsed side, the floating
// windows, the hidden panels, the side sizes and how often dock_end reported a change.  The
// document shows the saved text.  Floating windows have the window ids 501 .. 506.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

const ID_DOCK: u32 = 500; // floating windows: 501 ..
const ID_SAVE: u32 = 101;
const ID_LOAD: u32 = 102;
const ID_RESET: u32 = 103;
const ID_HISTORY: u32 = 104;
const ID_FILE0: u32 = 201; // .. 203: buttons in the Dateien panel
const ID_PROP: u32 = 210;
const ID_SEARCH_GO: u32 = 220;

const P_FILES: u32 = 11;
const P_OUTLINE: u32 = 12;
const P_PROPS: u32 = 13;
const P_OUTPUT: u32 = 14;
const P_SEARCH: u32 = 15;
const P_HISTORY: u32 = 16;
const PANELS: [u32; 6] = [P_FILES, P_OUTLINE, P_PROPS, P_OUTPUT, P_SEARCH, P_HISTORY];

const CMD_FOCUS0: u32 = 1; // .. 6: focus panel k
static CMDS: [Command; 6] = [
    Command::new(CMD_FOCUS0, "Dateien", 0, MOD_CTRL, b'1' as u32, 0),
    Command::new(CMD_FOCUS0 + 1, "Gliederung", 0, MOD_CTRL, b'2' as u32, 0),
    Command::new(CMD_FOCUS0 + 2, "Eigenschaften", 0, MOD_CTRL, b'3' as u32, 0),
    Command::new(CMD_FOCUS0 + 3, "Ausgabe", 0, MOD_CTRL, b'4' as u32, 0),
    Command::new(CMD_FOCUS0 + 4, "Suche", 0, MOD_CTRL, b'5' as u32, 0),
    Command::new(CMD_FOCUS0 + 5, "Verlauf", 0, MOD_CTRL, b'6' as u32, 0),
];

const DESC: AppDesc = AppDesc { title: "jfgui Docking", flags: AF_RESIZABLE, w: 960, h: 640, cmds: &CMDS, ..AppDesc::NONE };

fn title(p: u32) -> &'static str {
    match p {
        P_FILES => "Dateien",
        P_OUTLINE => "Gliederung",
        P_PROPS => "Eigenschaften",
        P_OUTPUT => "Ausgabe",
        P_SEARCH => "Suche",
        P_HISTORY => "Verlauf",
        _ => "?",
    }
}

#[derive(Default)]
struct Dock {
    dock: DockState,
    saved: String,
    changes: u32,
    clicked: String,
    /// --layout FILE: loaded at the start, written by Speichern
    file: Option<String>,
}

impl Dock {
    // "L: [Dateien]+Gliederung | R: [Eigenschaften] | U: [Ausgabe], [Suche] | F: - | V: Verlauf"
    fn describe(&self) -> String {
        let d = &self.dock;
        let group = |g: &Vec<u32>| {
            let v: Vec<String> = g.iter().map(|&p| if d.front(p) { format!("[{}]", title(p)) } else { String::from(title(p)) }).collect();
            v.join("+")
        };
        let mut parts: Vec<String> = Vec::new();
        for (s, name) in [(DOCK_LEFT, "L"), (DOCK_RIGHT, "R"), (DOCK_BOTTOM, "U")] {
            let gs: Vec<String> = d.groups(s).iter().map(group).collect();
            let zu = if d.collapsed(s) && !gs.is_empty() { "(zu)" } else { "" };
            parts.push(format!("{}{}: {}", name, zu, if gs.is_empty() { String::from("-") } else { gs.join(", ") }));
        }
        let fs: Vec<String> = d.floats().iter().map(group).collect();
        parts.push(format!("F: {}", if fs.is_empty() { String::from("-") } else { fs.join(", ") }));
        let hs: Vec<&str> = d.hidden_panels().iter().map(|&p| title(p)).collect();
        parts.push(format!("V: {}", if hs.is_empty() { String::from("-") } else { hs.join("+") }));
        parts.join(" | ")
    }

    fn panel(&mut self, ui: &mut Ui, p: u32) {
        ui.box_begin(0, BOX_V | PAD_12, 8);
        match p {
            DOCK_CENTER => {
                ui.text(TS_SUBTITLE, "Dokument");
                ui.text(TS_BODY | TX_SECONDARY, &format!("Klick: {}", if self.clicked.is_empty() { "-" } else { &self.clicked }));
                if self.saved.is_empty() {
                    ui.text(TS_CAPTION | TX_SECONDARY, "Kein Layout gespeichert");
                } else {
                    for l in self.saved.clone().lines() {
                        ui.text(TS_CAPTION, l);
                    }
                }
            }
            P_FILES => {
                for k in 0..3 {
                    let name = format!("Datei {}", k + 1);
                    if ui.button(ID_FILE0 + k, &name) & EV_ACT != 0 {
                        self.clicked = name;
                    }
                }
            }
            P_OUTLINE => {
                ui.text(TS_BODY, "1 Einleitung");
                ui.text(TS_BODY, "2 Hauptteil");
                ui.text(TS_BODY, "3 Schluss");
            }
            P_PROPS => {
                ui.text(TS_BODY, "Breite: 960");
                if ui.button(ID_PROP, "Eigenschaft") & EV_ACT != 0 {
                    self.clicked = String::from("Eigenschaft");
                }
            }
            P_OUTPUT => {
                ui.text(TS_BODY, "Bereit.");
            }
            P_SEARCH => {
                if ui.button(ID_SEARCH_GO, "Suchen") & EV_ACT != 0 {
                    self.clicked = String::from("Suchen");
                }
            }
            P_HISTORY => {
                ui.text(TS_BODY, "Geöffnet");
            }
            _ => {}
        }
        ui.box_end();
    }
}

impl App for Dock {
    fn init(&mut self, _ui: &mut Ui) -> bool {
        let args = plat::args();
        if let Some(i) = args.iter().position(|a| a == "--layout") {
            if let Some(f) = args.get(i + 1) {
                if let Some(d) = plat::read_file(f).and_then(|b| DockState::load(&String::from_utf8_lossy(&b))) {
                    self.dock = d;
                }
                self.file = Some(f.clone());
            }
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(SIZE_FILL, SIZE_FILL);
        ui.box_begin(0, BOX_V, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER | PAD_8, 8);
        if ui.button(ID_SAVE, "Speichern") & EV_ACT != 0 {
            self.saved = self.dock.save();
            if let Some(f) = &self.file {
                plat::write_file(f, self.saved.as_bytes());
            }
        }
        if ui.button(ID_LOAD, "Laden") & EV_ACT != 0 {
            if let Some(d) = DockState::load(&self.saved) {
                self.dock = d;
            }
        }
        if ui.button(ID_RESET, "Zurücksetzen") & EV_ACT != 0 {
            self.dock.reset();
        }
        let mut on = self.dock.visible(P_HISTORY) as i32;
        if ui.toggle_button(ID_HISTORY, "Verlauf", &mut on) & EV_ACT != 0 {
            self.dock.toggle(P_HISTORY);
        }
        ui.box_end();
        let panels = [
            DockPanel::new(P_FILES, "Dateien", IC_FOLDER).at(DOCK_LEFT, 0),
            DockPanel::new(P_OUTLINE, "Gliederung", IC_BULLETED_LIST).at(DOCK_LEFT, 0),
            DockPanel::new(P_PROPS, "Eigenschaften", IC_SETTINGS).at(DOCK_RIGHT, 0),
            DockPanel::new(P_OUTPUT, "Ausgabe", IC_DOCUMENT).at(DOCK_BOTTOM, 0).flags(DP_NOCLOSE),
            DockPanel::new(P_SEARCH, "Suche", IC_SEARCH).at(DOCK_BOTTOM, 1),
            DockPanel::new(P_HISTORY, "Verlauf", IC_HISTORY).at(DOCK_RIGHT, 1).flags(DP_HIDDEN),
        ];
        ui.next_size(SIZE_FILL, SIZE_FILL);
        ui.dock_begin(ID_DOCK, &mut self.dock, &panels);
        while let Some(p) = ui.dock_next(&mut self.dock) {
            self.panel(ui, p);
        }
        if ui.dock_end(&mut self.dock) {
            self.changes += 1;
        }
        ui.box_begin(0, BOX_H | PAD_8, 16);
        ui.text(TS_CAPTION, &self.describe());
        ui.box_end();
        let d = &self.dock;
        ui.box_begin(0, BOX_H | pad(8), 16);
        ui.text(TS_CAPTION, &format!("Größen: {}/{}/{}", d.size(DOCK_LEFT), d.size(DOCK_RIGHT), d.size(DOCK_BOTTOM)));
        ui.text(TS_CAPTION, &format!("Änderungen: {}", self.changes));
        ui.box_end();
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if (CMD_FOCUS0..CMD_FOCUS0 + 6).contains(&id) {
            self.dock.focus(PANELS[(id - CMD_FOCUS0) as usize]);
            return true;
        }
        false
    }
}

ui::main!(ui::launch(&DESC, Dock::default(), &style::STYLE));
