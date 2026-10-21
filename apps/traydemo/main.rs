// traydemo -- the notification-area symbol and system notifications (ui tray.rs, plat::tray):
// a symbol with a menu from the command table, notifications, and the events they send back;
// the taskbar button (ui taskbar.rs: progress, badge, a jump list task) and keeping the screen
// on (plat::inhibit) to try by hand
//
//   python build_rs.py run traydemo
//   build/win/traydemo.exe --script apps/traydemo/tests/demo.uis   (headless: plat::tray's dry
//                                                                 run prints what it would show)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use ui::*;

const ID_SHOW: u32 = 100;
const ID_HIDE: u32 = 101;
const ID_NOTE: u32 = 102;
const ID_PROGRESS: u32 = 103;
const ID_BADGE: u32 = 104;
const ID_BADGE_CLEAR: u32 = 105;
const ID_AWAKE: u32 = 106;
const ID_TASKS: u32 = 107;
const CMD_NOTE: u32 = 1;
const CMD_TICK: u32 = 2;
const CMD_HELLO: u32 = 3;
const CMD_QUIT: u32 = 4;

static CMDS: [Command; 3] = [
    Command::new(CMD_NOTE, "&Benachrichtigung senden", IC_INFO, 0, 0, 0),
    Command::new(CMD_TICK, "&Mit Häkchen", 0, 0, 0, 0),
    Command::new(CMD_QUIT, "B&eenden", 0, 0, 0, 0),
];
static M_MORE: [Mi; 1] = [Mi::Item(CMD_HELLO, "&Hallo", 0)];
static TRAY_MENU: [Mi; 6] = [Mi::Cmd(CMD_NOTE), Mi::Cmd(CMD_TICK), Mi::Sep, Mi::Sub("&Mehr", &M_MORE), Mi::Sep, Mi::Cmd(CMD_QUIT)];

const DESC: AppDesc = AppDesc {
    title: "jfgui Tray-Demo",
    flags: AF_RESIZABLE,
    w: 520,
    h: 340,
    min_w: 360,
    min_h: 240,
    cmds: &CMDS,
    ..AppDesc::NONE
};

#[derive(Default)]
struct Demo {
    shown: bool,
    tick: bool,
    notes: u32,
    last: String,
    progress: TaskbarProgress,
    badge: u32,
    awake: Option<plat::Inhibit>,
    tasks: bool,
}

// the next look of the progress: none -> 40 % -> paused -> error -> indeterminate -> none
fn next_progress(p: TaskbarProgress) -> TaskbarProgress {
    match p {
        TaskbarProgress::None => TaskbarProgress::Normal(0.4),
        TaskbarProgress::Normal(v) => TaskbarProgress::Paused(v),
        TaskbarProgress::Paused(v) => TaskbarProgress::Error(v),
        TaskbarProgress::Error(_) => TaskbarProgress::Indeterminate,
        TaskbarProgress::Indeterminate => TaskbarProgress::None,
    }
}

fn progress_text(p: TaskbarProgress) -> String {
    let pct = (p.value() * 100.0 + 0.5) as u32;
    match p {
        TaskbarProgress::None => String::from("aus"),
        TaskbarProgress::Normal(_) => format!("normal {pct} %"),
        TaskbarProgress::Paused(_) => format!("pausiert {pct} %"),
        TaskbarProgress::Error(_) => format!("Fehler {pct} %"),
        TaskbarProgress::Indeterminate => String::from("unbestimmt"),
    }
}

impl Demo {
    fn show(&mut self, ui: &mut Ui) {
        ui.cmd_state(CMD_TICK, if self.tick { CF_CHECKED } else { 0 });
        self.shown = ui.tray_set(None, "jfgui Tray-Demo", &TRAY_MENU);
        self.last = String::from(if self.shown { "Symbol gesetzt" } else { "Kein Infobereich auf diesem System" });
    }

    fn note(&mut self, ui: &mut Ui) {
        self.notes += 1;
        let id = ui.notify("jfgui Tray-Demo", &format!("Benachrichtigung Nummer {}", self.notes), None);
        self.last = if id != 0 { format!("Benachrichtigung {id} gesendet") } else { String::from("Benachrichtigung nicht möglich") };
    }
}

impl App for Demo {
    fn init(&mut self, ui: &mut Ui) -> bool {
        if plat::args().iter().any(|a| a == "--note") {
            self.note(ui); // started from the jump list's task
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(24), 12);
        ui.text(TS_TITLE, "Infobereich & Benachrichtigungen");
        let support = if ui.tray_supported() { "vorhanden" } else { "nicht vorhanden" };
        ui.text(TS_BODY | TX_SECONDARY, &format!("Infobereich: {support}"));
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        if ui.accent_button(ID_SHOW, "Symbol anzeigen") & EV_ACT != 0 {
            self.show(ui);
        }
        if ui.button(ID_HIDE, "Symbol entfernen") & EV_ACT != 0 {
            ui.tray_remove();
            self.shown = false;
            self.last = String::from("Symbol entfernt");
        }
        if ui.button(ID_NOTE, "Benachrichtigung senden") & EV_ACT != 0 {
            self.note(ui);
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        if ui.button(ID_PROGRESS, "Fortschritt") & EV_ACT != 0 {
            self.progress = next_progress(self.progress);
            let ok = ui.taskbar_progress(self.progress);
            self.last = String::from(if ok { "Fortschritt gesetzt" } else { "Keine Taskleiste auf diesem System" });
        }
        if ui.button(ID_BADGE, "Zähler +1") & EV_ACT != 0 {
            self.badge += 1;
            ui.taskbar_badge(self.badge);
        }
        if ui.button(ID_BADGE_CLEAR, "Zähler löschen") & EV_ACT != 0 {
            self.badge = 0;
            ui.taskbar_badge(0);
        }
        if ui.button(ID_AWAKE, "Wach halten") & EV_ACT != 0 {
            self.awake = match self.awake.take() {
                Some(_) => None, // dropping the guard lets the screen turn off again
                None => plat::inhibit("Tray-Demo: Bildschirm bleibt an", plat::INHIBIT_IDLE),
            };
            self.last = String::from(if self.awake.is_some() { "Bildschirm bleibt an" } else { "Standby wieder erlaubt" });
        }
        if ui.button(ID_TASKS, "Sprungliste") & EV_ACT != 0 {
            self.tasks = !self.tasks;
            let t = [JumpTask { title: String::from("Benachrichtigung senden"), args: alloc::vec![String::from("--note")], desc: String::from("Startet die Tray-Demo und sendet eine Benachrichtigung") }];
            let ok = ui.taskbar_tasks(if self.tasks { &t } else { &[] });
            self.last = String::from(match (ok, self.tasks) {
                (false, _) => "Keine Sprungliste auf diesem System",
                (true, true) => "Sprungliste gesetzt",
                (true, false) => "Sprungliste entfernt",
            });
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Fortschritt: {}, Zähler: {}, wach: {}", progress_text(self.progress), self.badge, if self.awake.is_some() { "ja" } else { "nein" }));
        ui.text(TS_BODY, &format!("Symbol: {}", if self.shown { "sichtbar" } else { "aus" }));
        ui.text(TS_BODY, &format!("Zuletzt: {}", self.last));
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, "Das Menü des Symbols kommt aus der Befehlstabelle; eine gewählte Zeile kommt als Befehl an, ein Klick auf das Symbol als UIC_TRAY, ein Klick auf eine Benachrichtigung als UIC_NOTE.");
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_TRAY => {
                self.last = String::from(if arg == TRAY_ACTIVATE { "Symbol angeklickt" } else { "Symbol doppelt angeklickt" });
                ui.bring_to_front();
            }
            UIC_NOTE => {
                self.last = format!("Benachrichtigung {arg} angeklickt");
                ui.bring_to_front();
            }
            UIC_NOTE_GONE => self.last = format!("Benachrichtigung {arg} geschlossen"),
            CMD_NOTE => self.note(ui),
            CMD_TICK => {
                self.tick = !self.tick;
                if self.shown {
                    self.show(ui);
                }
                self.last = format!("Häkchen {}", if self.tick { "an" } else { "aus" });
            }
            CMD_HELLO => self.last = String::from("Hallo aus dem Untermenü"),
            CMD_QUIT => ui.quit(),
            UIC_EXIT => {
                ui.tray_remove();
                ui.taskbar_progress(TaskbarProgress::None);
                ui.taskbar_badge(0);
            }
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, Demo::default(), &style::STYLE));
