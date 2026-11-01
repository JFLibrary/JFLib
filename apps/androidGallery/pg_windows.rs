// "Fenster & Dialoge": a tool window, a tool window with values, a modal dialog window with a nested
// one, a content dialog, a small tool window whose menus and lists leave it, a status bar
// (apps/gallery/sec_win.rs).  On a phone the windows lie inside the screen (lib/plat/web_windows.rs).

use alloc::format;
use alloc::string::String;

use ui::*;

const ID_TOOLBTN: u32 = 7000;
const ID_DLGBTN: u32 = 7001;
const ID_VALBTN: u32 = 7002;
const ID_POPBTN: u32 = 7003;
const ID_CONTENTBTN: u32 = 7004;
const ID_WIN_TOOL: u32 = 7100;
const ID_WIN_SLIDER: u32 = 7101;
const ID_WIN_TEXT: u32 = 7102;
const ID_WIN_COMBO: u32 = 7103;
const ID_WIN_RESET: u32 = 7104;
const ID_WIN_DLG: u32 = 7200;
const ID_WIN_DLG_CB: u32 = 7201;
const ID_WIN_MORE: u32 = 7202;
const ID_WIN_NESTED: u32 = 7300;
const ID_WIN_NEST_CB: u32 = 7301;
const ID_WIN_NEST_TB: u32 = 7302;
const ID_WIN_VAL: u32 = 7400;
const ID_WIN_VAL_ANG: u32 = 7401;
const ID_WIN_VAL_PROP: u32 = 7402;
const ID_WIN_POP: u32 = 7500;
const ID_WIN_POP_MENU: u32 = 7501;
const ID_WIN_POP_COMBO: u32 = 7502;
const ID_WIN_POP_TIP: u32 = 7503;
const ID_WIN_POP_ITEM: u32 = 7510; // .. 7521
const ID_DLG: u32 = 7600;

const RANGE_SIZE: Range = Range::new(1, 100, 1, 0);
const RF_ANG: RangeF = RangeF::new(-180.0, 180.0, 0.0, 2, 0);
const SHAPES: [&str; 4] = ["Rund", "Eckig", "Weich", "Hart"];
const POP_ROWS: [&str; 12] = [
    "Neu", "Öffnen", "Speichern", "Drucken", "Kopieren", "Einfügen", "Ausschneiden", "Löschen", "Drehen", "Spiegeln", "Zuschneiden",
    "Schließen",
];
const POP_COMBO: [&str; 20] = [
    "Normal", "Multiplizieren", "Additiv", "Farbig nachbelichten", "Farbig abwedeln", "Reflektieren", "Leuchten", "Überlagern",
    "Differenz", "Negation", "Aufhellen", "Abdunkeln", "Bildschirm", "Xor", "Hartes Licht", "Weiches Licht", "Farbton", "Sättigung",
    "Farbe", "Luminanz",
];

pub struct Windows {
    size: i32,
    text: String,
    combo: i32,
    tool_open: Shared<i32>,
    tool_rect: Shared<WinRect>,
    dlg_open: Shared<i32>,
    dlg_rect: Shared<WinRect>,
    dlg_cb: i32,
    result: i32,
    nested_open: Shared<i32>,
    nest_rect: Shared<WinRect>,
    nest_cb: i32,
    nest_text: String,
    nest_commits: i32,
    val_open: Shared<i32>,
    val_rect: Shared<WinRect>,
    ang: f64,
    pop_open: Shared<i32>,
    pop_rect: Shared<WinRect>,
    pop_sel: i32,
    pop_last: i32,
    content_open: Shared<i32>,
    content_result: i32,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            size: 20,
            text: String::new(),
            combo: 0,
            tool_open: Shared::new(0),
            tool_rect: Shared::new(WinRect::new(0, 0, 260, 0, 0)),
            dlg_open: Shared::new(0),
            dlg_rect: Shared::new(WinRect::new(0, 0, 340, 0, 0)),
            dlg_cb: 0,
            result: 0,
            nested_open: Shared::new(0),
            nest_rect: Shared::new(WinRect::new(0, 0, 300, 0, 0)),
            nest_cb: 0,
            nest_text: String::new(),
            nest_commits: 0,
            val_open: Shared::new(0),
            val_rect: Shared::new(WinRect::new(0, 0, 300, 0, 0)),
            ang: 45.0,
            pop_open: Shared::new(0),
            pop_rect: Shared::new(WinRect::new(24, 120, 220, 240, WRF_POS)),
            pop_sel: 0,
            pop_last: 0,
            content_open: Shared::new(0),
            content_result: 0,
        }
    }
}

impl Windows {
    pub fn page(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Werkzeug- und Dialogfenster sind eigene Fenster mit eigenem Zustand.");
        ui.next_size(SIZE_FILL, 44);
        ui.next_icon(IC_SETTINGS);
        let mut v = self.tool_open.get();
        ui.toggle_button(ID_TOOLBTN, "Werkzeugfenster", &mut v);
        self.tool_open.set(v);
        ui.next_size(SIZE_FILL, 44);
        let mut v = self.val_open.get();
        ui.toggle_button(ID_VALBTN, "Werte-Fenster", &mut v);
        self.val_open.set(v);
        ui.next_size(SIZE_FILL, 44);
        let mut v = self.pop_open.get();
        ui.toggle_button(ID_POPBTN, "Popups aus dem Fenster", &mut v);
        self.pop_open.set(v);
        ui.next_size(SIZE_FILL, 44);
        if ui.button(ID_DLGBTN, "Dialogfenster…") & EV_ACT != 0 {
            self.dlg_open.set(1);
        }
        ui.next_size(SIZE_FILL, 44);
        if ui.button(ID_CONTENTBTN, "Dialog auf der Seite…") & EV_ACT != 0 {
            self.content_open.set(1);
        }
        let shape = SHAPES.get(self.combo as usize).copied().unwrap_or("");
        ui.text(TS_BODY | TX_WRAP, &format!("Pinselgröße: {}   Name: {}   Form: {}", self.size, self.text, shape));
        ui.text(TS_BODY | TX_WRAP, &format!("Dialog: {}   Seitendialog: {}", self.result, self.content_result));
    }

    /// after the scrolling page: the status bar, the dialog, the windows
    pub fn windows(&mut self, ui: &mut Ui) {
        ui.statusbar_begin();
        ui.text(TS_CAPTION, "Bereit");
        ui.separator();
        ui.text(TS_CAPTION, &format!("Winkel: {}", ui.fmt_f64(self.ang, 0)));
        ui.statusbar_end();

        if ui.dialog_begin(ID_DLG, "Änderungen speichern?", &self.content_open) {
            ui.text(TS_BODY | TX_WRAP, "Möchten Sie die Änderungen an „Unbenannt“ speichern?");
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Ungespeicherte Änderungen gehen sonst verloren.");
            let r = ui.dialog_buttons(Some("Speichern"), None, Some("Abbrechen"));
            if r != 0 {
                self.content_result = r as i32;
            }
        }
        ui.dialog_end();

        if ui.window_begin(ID_WIN_TOOL, "Werkzeuge", Some(&self.tool_open), WF_TOOL | WF_AUTOHEIGHT, Some(&self.tool_rect)) {
            ui.box_begin(0, BOX_V | PAD_12, 8);
            ui.text(TS_BODY_STRONG, "Pinsel");
            ui.next_size(SIZE_FILL, 40);
            ui.slider(ID_WIN_SLIDER, &mut self.size, &RANGE_SIZE);
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("Größe: {} px", self.size));
            ui.next_size(SIZE_FILL, 44);
            ui.textbox(ID_WIN_TEXT, Some("Name des Pinsels"), &mut self.text, 48);
            ui.next_size(SIZE_FILL, 44);
            ui.combobox(ID_WIN_COMBO, &mut self.combo, &SHAPES);
            ui.next_size(SIZE_FILL, 44);
            if ui.button(ID_WIN_RESET, "Zurücksetzen") & EV_ACT != 0 {
                self.size = 20;
                self.combo = 0;
            }
            ui.tooltip("Setzt Größe und Form zurück");
            ui.box_end();
        }
        ui.window_end();

        if ui.window_begin(ID_WIN_DLG, "Bild bearbeiten", Some(&self.dlg_open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&self.dlg_rect)) {
            ui.text(TS_BODY | TX_WRAP, "Die Änderung wird auf das ganze Bild angewendet.");
            ui.next_size(0, 40);
            ui.checkbox(ID_WIN_DLG_CB, "Kanten glätten", &mut self.dlg_cb);
            ui.next_size(SIZE_FILL, 44);
            if ui.button(ID_WIN_MORE, "Weitere Optionen…") & EV_ACT != 0 {
                self.nested_open.set(1);
            }
            if ui.window_begin(ID_WIN_NESTED, "Weitere Optionen", Some(&self.nested_open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&self.nest_rect)) {
                ui.next_size(0, 40);
                ui.checkbox(ID_WIN_NEST_CB, "Farbprofil einbetten", &mut self.nest_cb);
                ui.next_size(SIZE_FILL, 44);
                if ui.textbox(ID_WIN_NEST_TB, Some("Profilname"), &mut self.nest_text, 32) & EV_COMMIT != 0 {
                    self.nest_commits += 1;
                }
                ui.text(TS_CAPTION | TX_SECONDARY, &format!("Übernommen: {}", self.nest_commits));
                ui.dialog_buttons(None, None, Some("Schließen"));
            }
            ui.window_end();
            let r = ui.dialog_buttons(Some("OK"), None, Some("Abbrechen"));
            if r != 0 {
                self.result = r as i32;
            }
        }
        ui.window_end();

        if ui.window_begin(ID_WIN_VAL, "Werte", Some(&self.val_open), WF_TOOL | WF_AUTOHEIGHT, Some(&self.val_rect)) {
            ui.box_begin(0, BOX_V | PAD_12, 8);
            ui.prop_slider_f(ID_WIN_VAL_PROP, "Winkel", &mut self.ang, &RF_ANG, Some(0.0));
            ui.angle(ID_WIN_VAL_ANG, &mut self.ang, 0);
            ui.box_end();
        }
        ui.window_end();

        if ui.window_begin(ID_WIN_POP, "Popups", Some(&self.pop_open), WF_TOOL, Some(&self.pop_rect)) {
            ui.box_begin(0, BOX_V | PAD_12, 8);
            ui.next_size(SIZE_FILL, 44);
            ui.dropdown_button(ID_WIN_POP_MENU, "Menü");
            if ui.flyout_begin(ID_WIN_POP_MENU, 0) {
                for (k, row) in POP_ROWS.iter().enumerate() {
                    ui.next_size(200, 44);
                    if ui.menu_item(ID_WIN_POP_ITEM + k as u32, row) & EV_ACT != 0 {
                        self.pop_last = k as i32 + 1;
                    }
                }
            }
            ui.flyout_end();
            ui.next_size(SIZE_FILL, 44);
            ui.combobox(ID_WIN_POP_COMBO, &mut self.pop_sel, &POP_COMBO);
            ui.next_size(SIZE_FILL, 44);
            ui.button(ID_WIN_POP_TIP, "Hinweis");
            ui.tooltip("Ein Tooltip, der über den unteren Rand des Fensters hinausragt");
            let last = if self.pop_last == 0 { "–" } else { POP_ROWS[(self.pop_last - 1) as usize] };
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("Zuletzt: {last}"));
            ui.box_end();
        }
        ui.window_end();
    }
}
