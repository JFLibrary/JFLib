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
// Verlauf; "Gruppe" gives Dateien and Gliederung DP_GROUP (their group moves as a whole from
// its menu); "Alle zeigen" shows every panel in the order of PANELS (DockState::show, as an app
// brings its hidden tools back); Ctrl+1 .. Ctrl+6 focus a panel (DockState::focus).  The status line describes the
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
use ui::i18n::{t, tf, Catalog};

const ID_DOCK: u32 = 500; // floating windows: 501 ..
const ID_SAVE: u32 = 101;
const ID_LOAD: u32 = 102;
const ID_RESET: u32 = 103;
const ID_HISTORY: u32 = 104;
const ID_GROUP: u32 = 105; // "Gruppe": DP_GROUP for the left group
const ID_SHOWALL: u32 = 106; // "Alle zeigen": every panel shown, in the order of PANELS
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

const DESC: AppDesc = AppDesc {
    title: "JFLib Docking", // i18n
    flags: AF_RESIZABLE,
    w: 960,
    h: 640,
    cmds: &CMDS,
    ..AppDesc::NONE
};

fn title(p: u32) -> &'static str {
    match p {
        P_FILES => t("Dateien"), // i18n
        P_OUTLINE => t("Gliederung"), // i18n
        P_PROPS => t("Eigenschaften"), // i18n
        P_OUTPUT => t("Ausgabe"), // i18n
        P_SEARCH => t("Suche"), // i18n
        P_HISTORY => t("Verlauf"), // i18n
        _ => "?",
    }
}

#[derive(Default)]
struct Dock {
    dock: DockState,
    saved: String,
    changes: u32,
    clicked: String,
    /// "Gruppe": Dateien and Gliederung move as one group (DP_GROUP)
    group: i32,
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
        for (s, name) in [(DOCK_LEFT, t("L")), (DOCK_RIGHT, t("R")), (DOCK_BOTTOM, t("U"))] {
            let gs: Vec<String> = d.groups(s).iter().map(group).collect();
            let zu = if d.collapsed(s) && !gs.is_empty() { t("(zu)") } else { "" };
            parts.push(format!("{}{}: {}", name, zu, if gs.is_empty() { String::from("-") } else { gs.join(", ") }));
        }
        let fs: Vec<String> = d.floats().iter().map(group).collect();
        parts.push(format!("{}: {}", t("F"), if fs.is_empty() { String::from("-") } else { fs.join(", ") }));
        let hs: Vec<&str> = d.hidden_panels().iter().map(|&p| title(p)).collect();
        parts.push(format!("{}: {}", t("V"), if hs.is_empty() { String::from("-") } else { hs.join("+") }));
        parts.join(" | ")
    }

    fn panel(&mut self, ui: &mut Ui, p: u32) {
        ui.box_begin(0, BOX_V | PAD_12, 8);
        match p {
            DOCK_CENTER => {
                ui.text(TS_SUBTITLE, t("Dokument"));
                ui.text(TS_BODY | TX_SECONDARY, &tf("Klick: {}", &[if self.clicked.is_empty() { "-" } else { self.clicked.as_str() }]));
                if self.saved.is_empty() {
                    ui.text(TS_CAPTION | TX_SECONDARY, t("Kein Layout gespeichert"));
                } else {
                    for l in self.saved.clone().lines() {
                        ui.text(TS_CAPTION, l);
                    }
                }
            }
            P_FILES => {
                for k in 0..3 {
                    let name = tf("Datei {}", &[&format!("{}", k + 1)]);
                    if ui.button(ID_FILE0 + k, &name) & EV_ACT != 0 {
                        self.clicked = name;
                    }
                }
            }
            P_OUTLINE => {
                ui.text(TS_BODY, t("1 Einleitung"));
                ui.text(TS_BODY, t("2 Hauptteil"));
                ui.text(TS_BODY, t("3 Schluss"));
            }
            P_PROPS => {
                ui.text(TS_BODY, t("Breite: 960"));
                if ui.button(ID_PROP, t("Eigenschaft")) & EV_ACT != 0 {
                    self.clicked = String::from(t("Eigenschaft"));
                }
            }
            P_OUTPUT => {
                ui.text(TS_BODY, t("Bereit."));
            }
            P_SEARCH => {
                if ui.button(ID_SEARCH_GO, t("Suchen")) & EV_ACT != 0 {
                    self.clicked = String::from(t("Suchen"));
                }
            }
            P_HISTORY => {
                ui.text(TS_BODY, t("Geöffnet"));
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
        if ui.button(ID_SAVE, t("Speichern")) & EV_ACT != 0 {
            self.saved = self.dock.save();
            if let Some(f) = &self.file {
                plat::write_file(f, self.saved.as_bytes());
            }
        }
        if ui.button(ID_LOAD, t("Laden")) & EV_ACT != 0 {
            if let Some(d) = DockState::load(&self.saved) {
                self.dock = d;
            }
        }
        if ui.button(ID_RESET, t("Zurücksetzen")) & EV_ACT != 0 {
            self.dock.reset();
        }
        let mut on = self.dock.visible(P_HISTORY) as i32;
        if ui.toggle_button(ID_HISTORY, t("Verlauf"), &mut on) & EV_ACT != 0 {
            self.dock.toggle(P_HISTORY);
        }
        ui.toggle_button(ID_GROUP, t("Gruppe"), &mut self.group);
        if ui.button(ID_SHOWALL, t("Alle zeigen")) & EV_ACT != 0 {
            for p in PANELS {
                self.dock.show(p);
            }
        }
        ui.box_end();
        let grp = if self.group != 0 { DP_GROUP } else { 0 };
        let panels = [
            DockPanel::new(P_FILES, title(P_FILES), IC_FOLDER).at(DOCK_LEFT, 0).flags(grp),
            DockPanel::new(P_OUTLINE, title(P_OUTLINE), IC_BULLETED_LIST).at(DOCK_LEFT, 0).flags(grp),
            DockPanel::new(P_PROPS, title(P_PROPS), IC_SETTINGS).at(DOCK_RIGHT, 0),
            DockPanel::new(P_OUTPUT, title(P_OUTPUT), IC_DOCUMENT).at(DOCK_BOTTOM, 0).flags(DP_NOCLOSE),
            DockPanel::new(P_SEARCH, title(P_SEARCH), IC_SEARCH).at(DOCK_BOTTOM, 1),
            DockPanel::new(P_HISTORY, title(P_HISTORY), IC_HISTORY).at(DOCK_RIGHT, 1).flags(DP_HIDDEN),
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
        ui.text(TS_CAPTION, &tf("Größen: {}/{}/{}", &[&format!("{}", d.size(DOCK_LEFT)), &format!("{}", d.size(DOCK_RIGHT)), &format!("{}", d.size(DOCK_BOTTOM))]));
        ui.text(TS_CAPTION, &tf("Änderungen: {}", &[&format!("{}", self.changes)]));
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

ui::main!({
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")).with_library_languages());
    ui::launch(&DESC.translated(), Dock::default(), &style::STYLE)
});
