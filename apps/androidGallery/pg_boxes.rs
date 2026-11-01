// "Container": tabs, an expander, closable info bars, a horizontal scroll view, a scroll bar on its
// own, toasts (apps/gallery/sec_list.rs: gal_containers).

use alloc::format;

use ui::*;

const ID_TABS: u32 = 5000; // tabs 5001 .. 5003
const ID_EXP: u32 = 5010;
const ID_EXP_CB: u32 = 5011;
const ID_IB0: u32 = 5020; // info bars 5020, 5022, 5024, 5026 (close buttons + 1)
const ID_IB_RESET: u32 = 5030;
const ID_TOAST: u32 = 5031;
const ID_HSCROLL: u32 = 5040;
const ID_SBAR: u32 = 5041;
const ID_HOLD: u32 = 5042;
const ID_DECLINE: u32 = 5043;
const RANGE: Range = Range::new(0, 100, 1, 0);
const ID_CHIP0: u32 = 5050; // .. 5069
const TAB_NAMES: [&str; 3] = ["Aufklappen", "Hinweise", "Rollen"];
const IB_TITLES: [&str; 4] = ["Hinweis", "Gespeichert", "Achtung", "Fehler"];
const IB_MSGS: [&str; 4] = [
    "Eine neue Version ist verfügbar.",
    "Das Bild wurde gespeichert.",
    "Der Akku ist fast leer.",
    "Die Datei konnte nicht geöffnet werden.",
];

#[derive(Default)]
pub struct Boxes {
    tab: i32,
    exp_open: i32,
    exp_cb: i32,
    closed: u32,
    sbar: i32,
    chips: i32,
    hold: i32,
    decline: i32,
}

impl Boxes {
    pub fn page(&mut self, ui: &mut Ui) {
        ui.next_size(SIZE_FILL, 44);
        ui.tabs(ID_TABS, &mut self.tab, &TAB_NAMES);
        match self.tab {
            1 => {
                for i in 0..4u32 {
                    if self.closed & (1 << i) != 0 {
                        continue;
                    }
                    if ui.infobar(ID_IB0 + i * 2, i, IB_TITLES[i as usize], IB_MSGS[i as usize]) & EV_ACT != 0 {
                        self.closed |= 1 << i;
                    }
                }
                ui.next_size(SIZE_FILL, 44);
                if ui.button(ID_IB_RESET, "Alle Hinweise zeigen") & EV_ACT != 0 {
                    self.closed = 0;
                }
                ui.next_size(SIZE_FILL, 44);
                if ui.button(ID_TOAST, "Kurze Meldung") & EV_ACT != 0 {
                    ui.toast(IB_INFO, "Meldung", "Sie verschwindet von selbst.", 4000);
                }
            }
            2 => {
                ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Waagerecht wischen rollt die Reihe.");
                ui.next_size(SIZE_FILL, 72);
                ui.scroll_begin(ID_HSCROLL, SC_H | BOX_CARD | PAD_8);
                for i in 0..20u32 {
                    ui.next_size(96, 44);
                    if ui.button(ID_CHIP0 + i, &format!("Knopf {}", i + 1)) & EV_ACT != 0 {
                        self.chips = i as i32 + 1;
                    }
                    ui.spacer(8);
                }
                ui.scroll_end();
                ui.text(TS_BODY, &format!("Zuletzt: Knopf {}", self.chips));
                if !ui.touch_mode() {
                    ui.next_size(SIZE_FILL, 0);
                    ui.scrollbar(ID_SBAR, &mut self.sbar, 1000, 250, false);
                    ui.text(TS_BODY, &format!("Rollleiste: {}", self.sbar));
                } else {
                    // (a finger gets no scroll bars) what a widget may ask of a finger: ui.touch_take
                    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Erst nach langem Drücken, zwei Finger gehören ihm:");
                    ui.next_size(SIZE_FILL, 40);
                    ui.slider(ID_HOLD, &mut self.hold, &RANGE);
                    ui.touch_take(TF_H | TF_HOLD | TF_TWO);
                    ui.text(TS_BODY, &format!("Finger darauf: {}", ui.touch_fingers().len()));
                    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Nimmt nur Tippen:");
                    ui.next_size(SIZE_FILL, 40);
                    ui.slider(ID_DECLINE, &mut self.decline, &RANGE);
                    ui.touch_take(TF_DECLINE);
                }
            }
            _ => {
                if ui.expander_begin(ID_EXP, "Erweiterte Einstellungen", &mut self.exp_open) != 0 {
                    ui.next_size(0, 40);
                    ui.checkbox(ID_EXP_CB, "Automatisch speichern", &mut self.exp_cb);
                    ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, "Der Inhalt erscheint, wenn der Expander offen ist.");
                }
                ui.expander_end();
            }
        }
    }
}
