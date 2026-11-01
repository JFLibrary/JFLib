// "Randfälle": controls in the screen's corners (apps/gallery/sec_edge.rs): their menus, lists, the
// calendar, the tooltip and the context menu with a submenu must stay on the screen: below or to
// the right when there is room, above or to the left when not.

use alloc::format;
use alloc::string::String;

use ui::*;

const ID_MENU: u32 = 8000; // top right
const ID_ROW: u32 = 8001; // .. 8006
const ID_LEFT: u32 = 8010; // top left
const ID_COMBO: u32 = 8011; // bottom left
const ID_DATE: u32 = 8012; // bottom, in the middle
const ID_TIP: u32 = 8013; // bottom right
const ID_CTX0: u32 = 8020; // context menu rows .. 8025
const ROWS: [&str; 6] = ["Ausschneiden", "Kopieren", "Einfügen", "Duplizieren", "Umbenennen", "Löschen"];
const COMBO: [&str; 12] =
    ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
const CTX_NAMES: [&str; 6] = ["Öffnen", "Umbenennen", "Eigenschaften", "Nach vorne", "Nach hinten", "Ganz nach hinten"];
static M_MORE: [Mi; 3] = [Mi::Item(ID_CTX0 + 3, "Nach vorne", 0), Mi::Item(ID_CTX0 + 4, "Nach hinten", 0), Mi::Item(ID_CTX0 + 5, "Ganz nach hinten", 0)];
static M_CTX: [Mi; 5] = [
    Mi::Item(ID_CTX0, "Öffnen", 0),
    Mi::Item(ID_CTX0 + 1, "Umbenennen", 0),
    Mi::Sep,
    Mi::Sub("Anordnen", &M_MORE),
    Mi::Item(ID_CTX0 + 2, "Eigenschaften", IC_INFO),
];

pub struct Edge {
    combo: i32,
    date: (i32, u32, u32),
    last: String,
}

impl Default for Edge {
    fn default() -> Self {
        Self { combo: 0, date: (2026, 11, 1), last: String::new() }
    }
}

impl Edge {
    /// the whole space below the bar
    pub fn page(&mut self, ui: &mut Ui) {
        ui.next_size(SIZE_FILL, SIZE_FILL);
        ui.box_begin(0, BOX_V | PAD_12, 8);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(72, 44);
        ui.button(ID_LEFT, "Links");
        ui.tooltip("Ein breiter Tooltip am linken Rand des Bildschirms");
        ui.next_size(SIZE_FILL, 0);
        ui.text(TS_BODY | TX_ELLIPSIS | TX_SECONDARY, &format!("Zuletzt: {}", if self.last.is_empty() { "–" } else { &self.last }));
        ui.next_size(88, 44);
        ui.dropdown_button(ID_MENU, "Menü");
        if ui.flyout_begin(ID_MENU, FL_BELOW | FL_ALIGN_END) {
            for (k, row) in ROWS.iter().enumerate() {
                ui.next_size(200, 44);
                if ui.menu_item(ID_ROW + k as u32, row) & EV_ACT != 0 {
                    self.last = String::from(*row);
                }
            }
        }
        ui.flyout_end();
        ui.box_end();
        ui.spacer(-1);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY | TX_CENTER, "Lange drücken öffnet auf „Hinweis“ das Kontextmenü.");
        ui.spacer(-1);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(SIZE_FILL, 44);
        if ui.combobox(ID_COMBO, &mut self.combo, &COMBO) & EV_ACT != 0 {
            self.last = String::from(COMBO[self.combo as usize]);
        }
        let (mut y, mut m, mut d) = self.date;
        ui.next_size(SIZE_FILL, 44);
        if ui.date_picker(ID_DATE, &mut y, &mut m, &mut d) & EV_ACT != 0 {
            self.last = format!("{d}.{m}.{y}");
        }
        self.date = (y, m, d);
        ui.next_size(88, 44);
        ui.button(ID_TIP, "Hinweis");
        ui.tooltip("Ein Tooltip am unteren rechten Rand des Bildschirms");
        ui.context_menu(&M_CTX);
        ui.box_end();
        ui.box_end();
    }

    pub fn command(&mut self, id: u32) -> bool {
        if (ID_CTX0..ID_CTX0 + 6).contains(&id) {
            self.last = String::from(CTX_NAMES[(id - ID_CTX0) as usize]);
            return true;
        }
        false
    }
}
