// sec_win -- the gallery's windows section, status bar, content dialog and windows
//   [apps/gallery/src/main.s: gal_win_section, gal_statusbar, gal_dialog, gal_windows,
//    gal_val_window, gal_pop_window, the window test hooks of gal_cmd]
//
// The windows' open flags and WINRECTs are ui::Shared values: the library keeps them like
// the asm keeps the addresses (close button, Esc, placement by the user).

use alloc::format;
use alloc::string::String;

use ui::*;

use crate::{cap, Gal};

// windows (window agent)
const ID_WIN_TOOL: u32 = 7600; // tool window "Werkzeuge"
const ID_WIN_SLIDER: u32 = 7601;
const ID_WIN_TEXT: u32 = 7602;
const ID_WIN_COMBO: u32 = 7603;
const ID_WIN_RESET: u32 = 7604;
const ID_WIN_DLG: u32 = 7620; // modal dialog window "Bild bearbeiten"
const ID_WIN_DLG_CB: u32 = 7621;
const ID_WIN_MORE: u32 = 7622;
const ID_WIN_NESTED: u32 = 7630; // dialog window opened by the dialog
const ID_WIN_NEST_CB: u32 = 7631;
const ID_WIN_NEST_TB: u32 = 7632; // text box in the nested dialog (Enter commits, U3)
const ID_WIN_TOOLBTN: u32 = 7640; // main window: toggle for the tool window
const ID_WIN_DLGBTN: u32 = 7641; // main window: opens the dialog
const CMD_WIN_DUP: u32 = 9101; // test hook: the tool window repeats id 100 (duplicate id)
const CMD_WIN_DESTROY: u32 = 9102; // test hook: close the tool window and free it
const CMD_WIN_FOCUS: u32 = 9103; // test hook: focus(the tool window's text box)
const CMD_WIN_POP: u32 = 9104; // (U3) test hook: toggle the small tool window "Popups"
const ID_WIN_POP: u32 = 7760; // (U3) tool window "Popups": menus / lists leave it
const ID_WIN_POP_MENU: u32 = 7761; // drop-down button with a long menu (popup id)
const ID_WIN_POP_ITEM: u32 = 7770; // its rows 7770..7781
const ID_WIN_POP_COMBO: u32 = 7762; // combo box with 20 entries
const ID_WIN_POP_TIP: u32 = 7763; // button with a tooltip at the window's bottom
const ID_WIN_VAL: u32 = 7750; // tool window "Werte"
const ID_WIN_VAL_ANG: u32 = 7751;
const ID_WIN_VAL_PROP: u32 = 7752;
const ID_BTN: u32 = 100; // the main window's "Standard" button
const ID_DLG: u32 = 7430; // the content dialog (containers section)

const RANGE_WIN_SIZE: Range = Range::new(1, 100, 1, 0);
const RF_ANG: RangeF = RangeF::new(-180.0, 180.0, 0.0, 2, 0);
const POP_ROWS: [&str; 12] = [
    "Neu", "Öffnen", "Speichern", "Drucken", "Kopieren", "Einfügen", "Ausschneiden", "Löschen", "Drehen", "Spiegeln",
    "Zuschneiden", "Schließen",
];
const POP_COMBO: [&str; 20] = [
    "Normal", "Multiplizieren", "Additiv", "Farbig nachbelichten", "Farbig abwedeln", "Reflektieren", "Leuchten",
    "Überlagern", "Differenz", "Negation", "Aufhellen", "Abdunkeln", "Bildschirm", "Xor", "Hartes Licht",
    "Weiches Licht", "Farbton", "Sättigung", "Farbe", "Luminanz",
];
const WIN_SHAPES: [&str; 4] = ["Rund", "Eckig", "Weich", "Hart"];

/// the section's state
pub(crate) struct S {
    size: i32,
    tool_rect: Shared<WinRect>,
    dlg_rect: Shared<WinRect>,
    nest_rect: Shared<WinRect>,
    pop_rect: Shared<WinRect>, // (U3)
    val_rect: Shared<WinRect>,
    tool_open: Shared<i32>,
    dlg_open: Shared<i32>,
    nested_open: Shared<i32>,
    pop_open: Shared<i32>,
    pop_sel: i32,
    pop_last: i32,
    pop_arg: bool,
    combo: i32,
    dlg_cb: i32,
    nest_cb: i32,
    nest_commits: i32,
    nest_text: String,
    result: i32,
    dup: bool,
    text: String,
}

impl Default for S {
    fn default() -> S {
        S {
            size: 20,
            tool_rect: Shared::new(WinRect::new(0, 0, 240, 0, 0)),
            dlg_rect: Shared::new(WinRect::new(0, 0, 380, 0, 0)),
            nest_rect: Shared::new(WinRect::new(0, 0, 300, 0, 0)),
            pop_rect: Shared::new(WinRect::new(40, 60, 180, 180, WRF_POS)),
            val_rect: Shared::new(WinRect::new(0, 0, 300, 0, 0)),
            tool_open: Shared::new(0),
            dlg_open: Shared::new(0),
            nested_open: Shared::new(0),
            pop_open: Shared::new(0),
            pop_sel: 0,
            pop_last: 0,
            pop_arg: false,
            combo: 0,
            dlg_cb: 0,
            nest_cb: 0,
            nest_commits: 0,
            nest_text: String::new(),
            result: 0,
            dup: false,
            text: String::new(),
        }
    }
}

/// gal_win_section: "Fenster" on the page -- open the windows, show their values
pub(crate) fn win_section(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.win;
    ui.text(TS_SUBTITLE, "Fenster");
    ui.text(TS_BODY | TX_SECONDARY, "Werkzeug- und Dialogfenster sind eigene Fenster mit eigenem Zustand.");
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_icon(IC_SETTINGS);
    let mut v = s.tool_open.get();
    ui.toggle_button(ID_WIN_TOOLBTN, "Werkzeugfenster", &mut v);
    s.tool_open.set(v);
    if ui.button(ID_WIN_DLGBTN, "Dialogfenster…") & EV_ACT != 0 {
        s.dlg_open.set(1);
    }
    let shape = WIN_SHAPES.get(s.combo as usize).copied().unwrap_or("");
    let line = cap(
        format!("Pinselgröße: {}   Name: {}   Form: {}   Dialog: {}", s.size, s.text, shape, s.result),
        192,
    );
    ui.text(TS_BODY, &line);
    ui.box_end();
}

/// gal_statusbar: status bar at the bottom of the window ("Auswahl: %d" of Gal::list_sel)
pub(crate) fn statusbar(g: &mut Gal, ui: &mut Ui) {
    ui.statusbar_begin();
    ui.text(TS_CAPTION, "Bereit");
    ui.separator();
    ui.text(TS_CAPTION, "200 Einträge");
    ui.separator();
    let t = cap(format!("Auswahl: {}", g.list_sel), 64);
    ui.text(TS_CAPTION, &t);
    ui.statusbar_end();
}

/// gal_dialog: the content dialog (Gal::dlg_open / dlg_result; always called, emits nothing
/// while closed)
pub(crate) fn dialog(g: &mut Gal, ui: &mut Ui) {
    if ui.dialog_begin(ID_DLG, "Änderungen speichern?", &g.dlg_open) {
        ui.text(TS_BODY, "Möchten Sie die Änderungen an „Unbenannt“ speichern?");
        ui.text(TS_BODY | TX_SECONDARY, "Ungespeicherte Änderungen gehen sonst verloren.");
        let r = ui.dialog_buttons(Some("Speichern"), None, Some("Abbrechen"));
        if r != 0 {
            g.dlg_result = r as i32;
        }
    }
    ui.dialog_end();
}

/// gal_windows: the tool window "Werkzeuge", the modal dialog window "Bild bearbeiten" and
/// its nested dialog (called at the end of the frame; each begin has its end)
pub(crate) fn windows(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.win;
    // ---- tool window: its height follows the content
    if ui.window_begin(ID_WIN_TOOL, "Werkzeuge", Some(&s.tool_open), WF_TOOL | WF_AUTOHEIGHT, Some(&s.tool_rect)) {
        ui.box_begin(0, BOX_V | PAD_12, 8);
        ui.text(TS_BODY_STRONG, "Pinsel");
        ui.slider(ID_WIN_SLIDER, &mut s.size, &RANGE_WIN_SIZE);
        let t = cap(format!("Größe: {} px", s.size), 64);
        ui.text(TS_CAPTION | TX_SECONDARY, &t);
        ui.textbox(ID_WIN_TEXT, Some("Name des Pinsels"), &mut s.text, 48);
        ui.combobox(ID_WIN_COMBO, &mut s.combo, &WIN_SHAPES);
        if ui.button(ID_WIN_RESET, "Zurücksetzen") & EV_ACT != 0 {
            s.size = 20;
            s.combo = 0;
        }
        ui.tooltip("Setzt Größe und Form zurück");
        if s.dup {
            ui.button(ID_BTN, "Standard (Kopie)"); // the same id as a main window button
        }
        ui.box_end();
    }
    ui.window_end();

    // ---- modal dialog window with a footer; it can open a nested dialog window
    if ui.window_begin(ID_WIN_DLG, "Bild bearbeiten", Some(&s.dlg_open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&s.dlg_rect)) {
        ui.text(TS_BODY | TX_WRAP, "Die Änderung wird auf das ganze Bild angewendet.");
        ui.checkbox(ID_WIN_DLG_CB, "Kanten glätten", &mut s.dlg_cb);
        if ui.button(ID_WIN_MORE, "Weitere Optionen…") & EV_ACT != 0 {
            s.nested_open.set(1);
        }
        if ui.window_begin(ID_WIN_NESTED, "Weitere Optionen", Some(&s.nested_open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&s.nest_rect)) {
            ui.checkbox(ID_WIN_NEST_CB, "Farbprofil einbetten", &mut s.nest_cb);
            // (U3) a single-line text box in a dialog without a primary button: Enter commits
            if ui.textbox(ID_WIN_NEST_TB, Some("Profilname"), &mut s.nest_text, 32) & EV_COMMIT != 0 {
                s.nest_commits += 1;
            }
            let t = cap(format!("Übernommen: {}", s.nest_commits), 64);
            ui.text(TS_CAPTION | TX_SECONDARY, &t);
            ui.dialog_buttons(None, None, Some("Schließen"));
        }
        ui.window_end();
        let r = ui.dialog_buttons(Some("OK"), None, Some("Abbrechen"));
        if r != 0 {
            s.result = r as i32;
        }
    }
    ui.window_end();
}

/// gal_val_window: tool window "Werte": a property row and an angle dial (Gal::win_val_open,
/// Gal::ang)
pub(crate) fn val_window(g: &mut Gal, ui: &mut Ui) {
    if ui.window_begin(ID_WIN_VAL, "Werte", Some(&g.win_val_open), WF_TOOL | WF_AUTOHEIGHT, Some(&g.win.val_rect)) {
        ui.box_begin(0, BOX_V | PAD_12, 8);
        ui.prop_slider_f(ID_WIN_VAL_PROP, "Winkel", &mut g.ang, &RF_ANG, Some(0.0));
        ui.angle(ID_WIN_VAL_ANG, &mut g.ang, 0);
        ui.box_end();
    }
    ui.window_end();
}

/// gal_pop_window: (U3) small tool window "Popups" (test hook 9104, --popups-window)
pub(crate) fn pop_window(g: &mut Gal, ui: &mut Ui) {
    let s = &mut g.win;
    if !s.pop_arg {
        s.pop_arg = true;
        // (tools/wincheck.py popups)
        if plat::args().iter().skip(1).any(|a| a == "--popups-window") {
            s.pop_open.set(1);
        }
    }
    if ui.window_begin(ID_WIN_POP, "Popups", Some(&s.pop_open), WF_TOOL, Some(&s.pop_rect)) {
        ui.box_begin(0, BOX_V | PAD_12, 8);
        ui.dropdown_button(ID_WIN_POP_MENU, "Menü");
        if ui.flyout_begin(ID_WIN_POP_MENU, 0) {
            for (k, row) in POP_ROWS.iter().enumerate() {
                if ui.menu_item(ID_WIN_POP_ITEM + k as u32, row) & EV_ACT != 0 {
                    s.pop_last = k as i32 + 1; // row + 1 (0 = none yet)
                }
            }
        }
        ui.flyout_end();
        ui.combobox(ID_WIN_POP_COMBO, &mut s.pop_sel, &POP_COMBO);
        ui.button(ID_WIN_POP_TIP, "Hinweis");
        ui.tooltip("Ein Tooltip, der über den unteren Rand des Fensters hinausragt");
        // the last menu row chosen
        let last = if s.pop_last == 0 { "–" } else { POP_ROWS[(s.pop_last - 1) as usize] };
        let t = cap(format!("Zuletzt: {}", last), 64);
        ui.text(TS_CAPTION | TX_SECONDARY, &t);
        ui.box_end();
    }
    ui.window_end();
}

/// gal_cmd's window test hooks -> handled: CMD_WIN_DUP (9101), CMD_WIN_DESTROY (9102),
/// CMD_WIN_POP (9104), CMD_WIN_FOCUS (9103)
pub(crate) fn cmd(g: &mut Gal, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    let s = &mut g.win;
    match id {
        CMD_WIN_DUP => s.dup = !s.dup,
        CMD_WIN_DESTROY => {
            s.tool_open.set(0);
            ui.window_destroy(ID_WIN_TOOL);
        }
        CMD_WIN_POP => s.pop_open.set(s.pop_open.get() ^ 1),
        CMD_WIN_FOCUS => ui.focus(ID_WIN_TEXT), // found in whichever window has it
        _ => return false,
    }
    true
}
