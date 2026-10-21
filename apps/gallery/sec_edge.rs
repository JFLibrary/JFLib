// sec_edge -- (§6) popups at the edges of the MAIN window: test hook 9105 (--popup-edges) shows
// only the menu bar and controls in the window's corners.  Their menus, lists, the calendar,
// the tooltip and the context menu (with a submenu) leave the window as OS popup windows and
// stay on the screen: below / to the right when there is room, above / to the left when not.
// Headless scripts give the virtual screen with `screen X Y W H` (apps/gallery/tests/
// popup_edges.uis, shots `desktop`); tools/wincheck.py edges and tools/x11check.py edges use the
// real windows, --popup-edges-open opens the combo box's list at the start
// (tools/wlproxy/popflip.py).

use alloc::format;

use ui::*;

use crate::{cap, Gal};

const CMD_EDGES: u32 = 9105; // test hook: the edge layout on / off
const ID_EDGE_MENU: u32 = 7800; // drop-down at the top right
const ID_EDGE_ROW: u32 = 7801; // its rows 7801 .. 7806
const ID_EDGE_COMBO: u32 = 7810; // combo box at the bottom left
const ID_EDGE_DATE: u32 = 7811; // date picker next to it
const ID_EDGE_TIP: u32 = 7812; // button at the bottom right: tooltip and context menu
const ID_EDGE_CTX0: u32 = 7820; // context menu rows 7820 .. 7825
const ID_EDGE_LEFT: u32 = 7813; // button at the top left: a wide tooltip
const ROWS: [&str; 6] = ["Ausschneiden", "Kopieren", "Einfügen", "Duplizieren", "Umbenennen", "Löschen"];
const COMBO: [&str; 12] = [
    "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember",
];
static M_MORE: [Mi; 3] =
    [Mi::Item(ID_EDGE_CTX0 + 3, "Nach vorne", 0), Mi::Item(ID_EDGE_CTX0 + 4, "Nach hinten", 0), Mi::Item(ID_EDGE_CTX0 + 5, "Ganz nach hinten", 0)];
static M_CTX: [Mi; 5] = [
    Mi::Item(ID_EDGE_CTX0, "Öffnen", 0),
    Mi::Item(ID_EDGE_CTX0 + 1, "Umbenennen", 0),
    Mi::Sep,
    Mi::Sub("Anordnen", &M_MORE),
    Mi::Item(ID_EDGE_CTX0 + 2, "Eigenschaften", IC_INFO),
];

/// the section's state
#[derive(Default)]
pub(crate) struct S {
    /// the edge layout is shown (test hook 9105, --popup-edges)
    pub(crate) on: bool,
    arg: bool,
    /// --popup-edges-open: the combo box's list opens with the first frame
    open: bool,
    combo: i32,
    date: (i32, u32, u32),
    /// the last row / day / context command chosen
    last: alloc::string::String,
}

/// the edge layout is on (the frame shows only it)
pub(crate) fn active(g: &mut Gal) -> bool {
    let s = &mut g.edge;
    if !s.arg {
        s.arg = true;
        s.on = plat::args().iter().skip(1).any(|a| a == "--popup-edges" || a == "--popup-edges-open");
        s.open = plat::args().iter().skip(1).any(|a| a == "--popup-edges-open");
        s.date = (2026, 10, 18);
    }
    s.on
}

/// the controls in the corners of the main window
pub(crate) fn frame(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.edge;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | PAD_12, 8);
    // top: a button at the left edge, the last choice, the drop-down at the right edge
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.button(ID_EDGE_LEFT, "Links");
    ui.tooltip("Ein breiter Tooltip am linken Rand des Hauptfensters");
    ui.next_size(-1, 0);
    let t = cap(format!("Zuletzt: {}", if s.last.is_empty() { "–" } else { &s.last }), 64);
    ui.text(TS_BODY | TX_SECONDARY, &t);
    ui.dropdown_button(ID_EDGE_MENU, "Menü");
    if ui.flyout_begin(ID_EDGE_MENU, FL_BELOW | FL_ALIGN_END) {
        for (k, row) in ROWS.iter().enumerate() {
            if ui.menu_item(ID_EDGE_ROW + k as u32, row) & EV_ACT != 0 {
                s.last = alloc::string::String::from(*row);
            }
        }
    }
    ui.flyout_end();
    ui.box_end();
    ui.spacer(-1);
    // bottom: combo box and calendar on the left, the button at the right edge
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    if s.open {
        s.open = false;
        ui.popup_open(ID_EDGE_COMBO);
    }
    if ui.combobox(ID_EDGE_COMBO, &mut s.combo, &COMBO) & EV_ACT != 0 {
        s.last = alloc::string::String::from(COMBO[s.combo as usize]);
    }
    let (mut y, mut m, mut d) = s.date;
    if ui.date_picker(ID_EDGE_DATE, &mut y, &mut m, &mut d) & EV_ACT != 0 {
        s.last = format!("{d}.{m}.{y}");
    }
    s.date = (y, m, d);
    ui.spacer(-1);
    ui.button(ID_EDGE_TIP, "Hinweis");
    ui.tooltip("Ein Tooltip am unteren rechten Rand des Hauptfensters");
    ui.context_menu(&M_CTX);
    ui.box_end();
    ui.box_end();
}

/// the test hook and the context menu's rows -> handled
pub(crate) fn cmd(g: &mut Gal, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
    let s = &mut g.edge;
    match id {
        CMD_EDGES => s.on = !s.on,
        _ if (ID_EDGE_CTX0..ID_EDGE_CTX0 + 6).contains(&id) => {
            let names = ["Öffnen", "Umbenennen", "Eigenschaften", "Nach vorne", "Nach hinten", "Ganz nach hinten"];
            s.last = alloc::string::String::from(names[(id - ID_EDGE_CTX0) as usize]);
        }
        _ => return false,
    }
    true
}
