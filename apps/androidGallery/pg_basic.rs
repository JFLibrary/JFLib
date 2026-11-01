// "Schaltflächen": the core widgets of the big gallery (apps/gallery/main.rs): buttons, the tool
// palette, selection, ranges, a card, for the finger: one column, 44 DIP targets.

use alloc::format;
use ui::*;

const ID_BTN: u32 = 1000;
const ID_ACCENT: u32 = 1001;
const ID_DISABLED: u32 = 1002;
const ID_SAVE: u32 = 1003;
const ID_TOGGLE: u32 = 1004;
const ID_ICONBTN: u32 = 1005;
const ID_TOOL0: u32 = 1010; // .. 1015
const ID_CB1: u32 = 1020;
const ID_CB2: u32 = 1021;
const ID_R0: u32 = 1022; // .. 1024
const ID_SWITCH: u32 = 1025;
const ID_SEG: u32 = 1026; // .. 1030
const ID_SLIDER: u32 = 1040;
const ID_VSLIDER: u32 = 1041;
const ID_PROGRESS: u32 = 1042;
const ID_THEME: u32 = 1043;

const RANGE_PCT: Range = Range::new(0, 100, 1, 0);
const RANGE_VERT: Range = Range::new(1, 50, 1, RG_VERT);
const TOOL_ICONS: [u32; 6] = [IC_PENCIL, IC_BRUSH, IC_ERASER, IC_DROP, IC_EYEDROPPER, IC_ZOOM];
const TOOL_NAMES: [&str; 6] = ["Stift", "Pinsel", "Radierer", "Füllen", "Farbauswahl", "Lupe"];
const COLORS: [&str; 3] = ["Rot", "Grün", "Blau"];
const SEGMENTS: [&str; 3] = ["Tag", "Woche", "Monat"];

pub struct Basic {
    clicks: i32,
    toggle: i32,
    tool: i32,
    cb1: i32,
    cb2: i32,
    radio: i32,
    switch: i32,
    seg: i32,
    slider: i32,
    vslider: i32,
}

impl Default for Basic {
    fn default() -> Self {
        Self { clicks: 0, toggle: 0, tool: 1, cb1: 1, cb2: 2, radio: 1, switch: 1, seg: 0, slider: 40, vslider: 12 }
    }
}

impl Basic {
    pub fn page(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY_STRONG, "Schaltflächen");
        ui.next_size(SIZE_FILL, 44);
        if ui.button(ID_BTN, "Standard") & EV_ACT != 0 {
            self.clicks += 1;
        }
        ui.next_size(SIZE_FILL, 44);
        if ui.accent_button(ID_ACCENT, "Akzent") & EV_ACT != 0 {
            self.clicks += 1;
        }
        ui.next_size(SIZE_FILL, 44);
        ui.next_flags(ST_DISABLED);
        ui.button(ID_DISABLED, "Deaktiviert");
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(48, 44);
        ui.icon_button(ID_SAVE, IC_SAVE, "Speichern");
        ui.next_size(SIZE_FILL, 44);
        ui.toggle_button(ID_TOGGLE, "Umschalten", &mut self.toggle);
        ui.next_size(SIZE_FILL, 44);
        ui.next_icon(IC_ADD);
        if ui.button(ID_ICONBTN, "Mit Icon") & EV_ACT != 0 {
            self.clicks += 1;
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Klicks: {}", self.clicks));

        ui.text(TS_BODY_STRONG, "Werkzeuge");
        ui.box_begin(0, BOX_H | BOX_WRAP, 4);
        for i in 0..6 {
            ui.next_size(48, 48);
            ui.tool_button(ID_TOOL0 + i as u32, TOOL_ICONS[i], TOOL_NAMES[i], &mut self.tool, i as i32);
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Werkzeug: {}", TOOL_NAMES[self.tool as usize]));

        ui.text(TS_BODY_STRONG, "Auswahl");
        ui.next_size(0, 40);
        ui.checkbox(ID_CB1, "Option A", &mut self.cb1);
        ui.next_size(0, 40);
        ui.checkbox(ID_CB2, "Unbestimmt", &mut self.cb2);
        for (i, name) in COLORS.iter().enumerate() {
            ui.next_size(0, 40);
            ui.radio(ID_R0 + i as u32, name, &mut self.radio, i as i32);
        }
        ui.next_size(0, 40);
        ui.switch(ID_SWITCH, "WLAN", &mut self.switch);
        ui.next_size(SIZE_FILL, 44);
        ui.segmented(ID_SEG, &mut self.seg, &SEGMENTS);

        ui.text(TS_BODY_STRONG, "Bereiche");
        ui.next_size(SIZE_FILL, 40);
        ui.slider(ID_SLIDER, &mut self.slider, &RANGE_PCT);
        ui.progress(ID_PROGRESS, self.slider, 100);
        ui.text(TS_BODY, &format!("Wert: {} %", self.slider));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.next_size(60, 140);
        ui.slider(ID_VSLIDER, &mut self.vslider, &RANGE_VERT);
        ui.touch_drag();
        ui.text(TS_BODY, &format!("Senkrecht: {}", self.vslider));
        ui.box_end();

        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 8);
        ui.text(TS_BODY_STRONG, "Karte");
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, "Karten gruppieren Inhalte auf einer leicht abgesetzten Fläche.");
        ui.separator();
        ui.next_size(SIZE_FILL, 44);
        if ui.button(ID_THEME, "Hell / Dunkel") & EV_ACT != 0 {
            let dark = ui.theme_dark();
            ui.theme_set(!dark as u32);
        }
        ui.box_end();
    }
}
