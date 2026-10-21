// gallery -- widget gallery: every jfguiLib control on one page  [apps/gallery/src/main.s, u2demo.s]
//
//   python build_rs.py app gallery
//   build/win/gallery.exe --shot g.png --theme light --scale 150
//   build/win/gallery.exe --script apps/gallery/tests/basic.uis
//
// main.rs: the frame (gal_frame), the commands (gal_cmd), the app record and the sections of
// the core widgets (buttons, tool palette, selection, ranges, card, ellipsis texts, wrapped
// texts).  The widget families fill their sections, called in the asm order:
//   sec_popup.rs  menu bar, "Menüs & Auswahl", tool row, menu / command ids    (ui-popup)
//   sec_edit.rs   text input, multi-line boxes, number boxes                   (ui-edit)
//   sec_list.rs   page scroll view, containers, values, colour wheel, lists    (ui-list)
//   sec_win.rs    windows section, status bar, content dialog, the windows     (ui-win)
//   sec_edge.rs   (§6) popups at the edges of the main window (test hook 9105)
// State shared by several sections lives in Gal; a section's own state in its module.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod sec_edge;
mod sec_edit;
mod sec_list;
mod sec_popup;
mod sec_win;

use alloc::format;
use alloc::string::String;

use ui::*;

const ID_BTN: u32 = 100;
const ID_ACCENT: u32 = 101;
const ID_DISABLED: u32 = 102;
const ID_SAVE: u32 = 103;
const ID_TOGGLE: u32 = 104;
const ID_ICONBTN: u32 = 105;
const ID_CB1: u32 = 110;
const ID_CB2: u32 = 111;
const ID_R1: u32 = 120;
const ID_R2: u32 = 121;
const ID_R3: u32 = 122;
const ID_SWITCH: u32 = 130;
const ID_SLIDER: u32 = 140;
const ID_VSLIDER: u32 = 141;
const ID_THEME: u32 = 150;
const ID_TOOL0: u32 = 160;
const ID_U2_CARD: u32 = 8600; // (U2) wrapped texts
const ID_U2_OPEN: u32 = 8620;
// grid box ("Raster"): a form of labels, fields and buttons with spans and alignment
const ID_GB: u32 = 8700;
const ID_GB_NAME: u32 = 8701;
const ID_GB_PICK: u32 = 8702;
const ID_GB_DESC: u32 = 8703;
const ID_GB_SAVE: u32 = 8704;
const ID_GB_SHARE: u32 = 8705;
const ID_GB_KEEP: u32 = 8706;
const ID_GB_OK: u32 = 8707;

const DESC: AppDesc = AppDesc {
    title: "jfgui Galerie",
    flags: AF_RESIZABLE,
    w: 900,
    h: 620,
    min_w: 480,
    min_h: 360,
    // (popup agent) the command table and the menu bar (gal_cmds, gal_menubar):
    // sec_popup.rs
    cmds: &sec_popup::CMDS,
    menubar: &sec_popup::MENUBAR,
    ..AppDesc::NONE
};

const RANGE_PCT: Range = Range::new(0, 100, 1, 0);
const RANGE_VERT: Range = Range::new(1, 50, 1, RG_VERT);
pub(crate) const TOOL_ICONS: [u32; 6] = [IC_PENCIL, IC_BRUSH, IC_ERASER, IC_DROP, IC_EYEDROPPER, IC_ZOOM];
pub(crate) const TOOL_NAMES: [&str; 6] = ["Stift", "Pinsel", "Radierer", "Füllen", "Farbauswahl", "Lupe"];

const S_LONG: &str = "Eine sehr lange Beschreibung, die nicht in die Breite passt";
const S_LONG2: &str = "Pinsel: Klicken und ziehen, um mit der Primärfarbe zu malen.";
const S_HINT: &str = "Farbpinsel: Klicken und ziehen, um mit der Primärfarbe zu malen, Rechtsklick für die Sekundärfarbe.";
const S_U2_ASK: &str = "„Unbenannt.png“ hat nicht gespeicherte Änderungen. Möchten Sie die Änderungen speichern, bevor Sie das Bild schließen?";
const S_U2_LONG: &str = "Donaudampfschifffahrtsgesellschaftskapitänsmützenabzeichen passen in keine Zeile.";
const S_U2_LINES: &str = "Erste Zeile\nZweite Zeile, länger als die Karte breit ist";
const S_U2_ROW: &str = "Das Bild wird zusammengefasst gespeichert; die Ebenen im Dokument bleiben erhalten.";
const S_U2_CUT: &str = "Dieser Hinweis ist länger als zwei Zeilen und wird deshalb am Ende mit Auslassungspunkten gekürzt.";
const S_U2_PAGE: &str = "Dieser Absatz ist so breit wie die Seite: in einem schmalen Fenster braucht er mehr Zeilen als in einem breiten, und die Höhe der Seite passt sich an, ohne dass die App etwas messen muss.";

/// fmt_buf(buf, cap, ...): the text cut to cap - 1 bytes (never inside a UTF-8 sequence)
pub(crate) fn cap(s: String, cap: usize) -> String {
    if s.len() < cap {
        return s;
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&s[..n])
}

/// the app state (the .data / .bss of main.s); state of several sections lives here, a
/// section's own state in its module's record
#[allow(dead_code)] // R5: the shared fields are used once the families fill their sections
pub(crate) struct Gal {
    cb1: i32,
    cb2: i32,
    radio: i32,
    switch: i32,
    slider: i32,
    vslider: i32,
    toggle: i32,
    clicks: i32,
    u2_open: String,
    gb_name: String,
    gb_desc: String,
    gb_opt: [i32; 3],
    // shared by several sections
    /// tool palette (core) and the tool row (popup)
    pub(crate) tool: i32,
    /// the containers list (list) and the status bar (win)
    pub(crate) list_sel: i32,
    /// the angle of the values section (list) and of the value window (win)
    pub(crate) ang: f64,
    /// "Werte-Fenster" toggle (list) and the value window (win)
    pub(crate) win_val_open: Shared<i32>, // ui-win: the library keeps the window's open flag
    /// the content dialog: opened in the containers (list), shown by gal_dialog (win)
    pub(crate) dlg_open: Shared<i32>, // ui-win: the library keeps the dialog's open flag
    pub(crate) dlg_result: i32,
    /// the last recent file chosen (popup) shown in the lists section (list)
    pub(crate) recent_last: String,
    pub(crate) pop: sec_popup::S,
    pub(crate) edit: sec_edit::S,
    pub(crate) list: sec_list::S,
    pub(crate) win: sec_win::S,
    pub(crate) edge: sec_edge::S,
}

impl Default for Gal {
    fn default() -> Gal {
        Gal {
            cb1: 1,
            cb2: 2,
            radio: 1,
            switch: 1,
            slider: 40,
            vslider: 12,
            toggle: 0,
            clicks: 0,
            u2_open: String::from("Ordner build: noch nicht geöffnet"),
            gb_name: String::new(),
            gb_desc: String::new(),
            gb_opt: [1, 0, 0],
            tool: 1,
            list_sel: 2,
            ang: 45.0,
            win_val_open: Shared::new(0),
            dlg_open: Shared::new(0),
            dlg_result: 0,
            recent_last: String::from("-"),
            pop: Default::default(),
            edit: Default::default(),
            list: Default::default(),
            win: Default::default(),
            edge: Default::default(),
        }
    }
}

impl App for Gal {
    // gal_frame: build the page
    fn frame(&mut self, ui: &mut Ui) {
        let g = self;
        sec_popup::menubar_top(g, ui); // (popup agent) menu bar above the page
        if sec_edge::active(g) {
            sec_edge::frame(g, ui); // (§6) only the controls in the window's corners
            return;
        }
        // the page scrolls, the status bar stays at the bottom (container agent)
        sec_list::page_begin(g, ui);
        ui.box_begin(0, BOX_V | PAD_24, 12);
        ui.text(TS_TITLE, "jfgui Galerie");
        ui.text(TS_BODY | TX_SECONDARY, "WinUI 3 Steuerelemente, geschrieben in purem x86-64 Assembly.");

        // ---- buttons
        ui.text(TS_SUBTITLE, "Schaltflächen");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.button(ID_BTN, "Standard") & EV_ACT != 0 {
            g.clicks += 1;
        }
        if ui.accent_button(ID_ACCENT, "Akzent") & EV_ACT != 0 {
            g.clicks += 1;
        }
        ui.next_flags(ST_DISABLED);
        ui.button(ID_DISABLED, "Deaktiviert");
        ui.icon_button(ID_SAVE, IC_SAVE, "Speichern");
        ui.toggle_button(ID_TOGGLE, "Umschalten", &mut g.toggle);
        ui.next_icon(IC_ADD);
        ui.button(ID_ICONBTN, "Mit Icon");
        ui.box_end();

        // ---- tool palette (like Kritzel's tools)
        ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
        for i in 0..6 {
            ui.tool_button(ID_TOOL0 + i as u32, TOOL_ICONS[i], TOOL_NAMES[i], &mut g.tool, i as i32);
        }
        ui.box_end();

        // ---- selection controls
        ui.text(TS_SUBTITLE, "Auswahl");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 24);
        ui.checkbox(ID_CB1, "Option A", &mut g.cb1);
        ui.checkbox(ID_CB2, "Unbestimmt", &mut g.cb2);
        ui.radio(ID_R1, "Rot", &mut g.radio, 0);
        ui.radio(ID_R2, "Grün", &mut g.radio, 1);
        ui.radio(ID_R3, "Blau", &mut g.radio, 2);
        ui.switch(ID_SWITCH, "WLAN", &mut g.switch);
        ui.box_end();

        // ---- ranges
        ui.text(TS_SUBTITLE, "Bereiche");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 24);
        ui.slider(ID_SLIDER, &mut g.slider, &RANGE_PCT);
        ui.progress(1, g.slider, 100);
        ui.next_size(60, 100);
        ui.slider(ID_VSLIDER, &mut g.vslider, &RANGE_VERT);
        // value text
        let t = cap(format!("Wert: {} %   Klicks: {}", g.slider, g.clicks), 128);
        ui.text(TS_BODY, &t);
        ui.box_end();

        // ---- text input (edit agent)
        sec_edit::text_input(g, ui);

        // ---- a card
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 4);
        ui.text(TS_BODY_STRONG, "Karte");
        ui.text(TS_CAPTION | TX_SECONDARY, "Karten gruppieren Inhalte auf einer leicht abgesetzten Fläche.");
        ui.separator();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY, "Design:");
        if ui.button(ID_THEME, "Hell / Dunkel") & EV_ACT != 0 {
            ui.cmd_run(ID_THEME, 0);
        }
        ui.box_end();
        ui.box_end();

        sec_popup::menus(g, ui); // (popup agent) menus, drop-downs, tooltips

        // ---- containers (container agent)
        sec_list::containers(g, ui);
        sec_win::win_section(g, ui); // (window agent) tool / dialog windows
        sec_list::values(g, ui); // (U1) "Werte & Farben"
        sec_list::wheel(g, ui); // (U1) colour wheel and picker
        sec_list::lists(g, ui); // (U1) "Listen": reorderable rows, MI_LIST
        gal_texts(g, ui); // (U1) "Text": ellipsis, multi-line boxes
        sec_popup::toolrow(g, ui); // (U1) flat drop-down, command states
        gal_u2(g, ui); // (U2) wrapped text, number boxes, open folder
        gal_gridbox(g, ui); // grid box: a form with spans and alignment

        ui.box_end();
        sec_list::page_end(g, ui); // page (container agent)
        sec_list::list_flyout(g, ui); // test hook 9061: a list flyout focused at once
        sec_win::statusbar(g, ui); // (container agent)
        sec_win::dialog(g, ui); // (container agent)
        sec_win::windows(g, ui); // (window agent) the windows themselves
        sec_win::val_window(g, ui); // (U1) tool window with value widgets
        sec_win::pop_window(g, ui); // (U3) popups that leave their window
    }

    // gal_cmd(id, arg) -> handled
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        let g = self;
        if sec_popup::menu_cmd(g, ui, id, arg) {
            return true; // (popup agent) commands
        }
        if sec_win::cmd(g, ui, id, arg) {
            return true; // (window agent) test hooks
        }
        if sec_edge::cmd(g, ui, id, arg) {
            return true; // (§6) the edge layout's hook and context menu
        }
        if sec_popup::u1_cmd(g, ui, id, arg) || sec_list::u1_cmd(g, ui, id, arg) {
            return true; // (U1) recent list, damage demo, command states, key names
        }
        if sec_edit::u2_cmd(g, ui, id, arg) {
            return true; // (U2) test hooks 9110 / 9111
        }
        if id == ID_THEME {
            let d = ui.theme_dark();
            ui.theme_set(!d as u32);
            return true;
        }
        false
    }
}

// gal_texts: "Text" -- ui_text with TX_ELLIPSIS in narrow boxes, multi-line text boxes
fn gal_texts(g: &mut Gal, ui: &mut Ui) {
    ui.text(TS_SUBTITLE, "Text");
    // ellipsis: the same sentence in three widths
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    ui.next_size(120, 0);
    ui.text(TS_BODY | TX_ELLIPSIS, S_LONG);
    ui.next_size(220, 0);
    ui.text(TS_CAPTION | TX_ELLIPSIS | TX_SECONDARY, S_LONG2);
    ui.next_size(400, 0);
    ui.text(TS_BODY | TX_ELLIPSIS, "Kurzer Text");
    ui.box_end();
    // a status-bar-like row: the hint takes the rest of the width and is cut when needed
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.icon(IC_BRUSH, 16);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_ELLIPSIS, S_HINT);
    ui.text(TS_CAPTION, "800 × 600");
    ui.box_end();
    sec_edit::text_boxes(g, ui); // multi-line boxes (edit agent)
}

// gal_u2: "Umbruch, Zahlen, Ordner" (U2) -- ui_text TX_WRAP in a fixed-width card (plain,
// centred, right-aligned with '\n', a word longer than a line), in a row next to an icon,
// cut to two lines with TX_ELLIPSIS; number boxes; open_folder  [u2demo.s]
// gal_gridbox: "Raster" -- labels (natural column), fields (fill), buttons (120 DIP);
// the description spans two columns, "Optionen" two rows, OK sits at the end of its cell
fn gal_gridbox(g: &mut Gal, ui: &mut Ui) {
    ui.text(TS_SUBTITLE, "Raster");
    ui.next_size(480, 0);
    ui.gridbox_begin(ID_GB, &[0, -1, 120], BOX_CARD | pad(16), 8);
    ui.text(TS_BODY, "Name");
    ui.textbox(ID_GB_NAME, Some("Vorname Nachname"), &mut g.gb_name, 64);
    ui.button(ID_GB_PICK, "Wählen …");
    ui.text(TS_BODY, "Beschreibung");
    ui.next_span(2, 1);
    ui.textbox(ID_GB_DESC, Some("Worum es geht"), &mut g.gb_desc, 256);
    ui.next_span(1, 2);
    ui.next_align(AL_START, AL_START);
    ui.text(TS_BODY, "Optionen");
    let [a, b, c] = &mut g.gb_opt;
    ui.checkbox(ID_GB_SAVE, "Speichern", a);
    ui.checkbox(ID_GB_SHARE, "Teilen", b);
    ui.checkbox(ID_GB_KEEP, "Archivieren", c);
    ui.next_align(AL_END, AL_DEFAULT);
    ui.accent_button(ID_GB_OK, "OK");
    ui.gridbox_end();
}

fn gal_u2(g: &mut Gal, ui: &mut Ui) {
    ui.text(TS_SUBTITLE, "Umbruch, Zahlen, Ordner");
    // a 260 DIP card with wrapped texts
    ui.next_size(260, 0);
    ui.box_begin(ID_U2_CARD, BOX_V | BOX_CARD | pad(12), 8);
    ui.text(TS_BODY | TX_WRAP, S_U2_ASK);
    ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY | TX_CENTER, S_U2_LONG);
    ui.text(TS_BODY | TX_WRAP | TX_RIGHT, S_U2_LINES);
    ui.box_end();
    // a row: icon, 200 DIP of wrapped text, a label after it
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.icon(IC_INFO, 16);
    ui.next_size(200, 0);
    ui.text(TS_BODY | TX_WRAP, S_U2_ROW);
    ui.text(TS_CAPTION, "danach");
    ui.box_end();
    // two lines at most: the rest is cut with "…"
    ui.next_size(180, 40);
    ui.text(TS_BODY | TX_WRAP | TX_ELLIPSIS, S_U2_CUT);
    // number boxes (int and f64) (edit agent)
    sec_edit::u2_numbers(g, ui);
    // open a folder in the file manager
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_FOLDER_OPEN);
    if ui.button(ID_U2_OPEN, "Ordner öffnen") & EV_ACT != 0 {
        let r = ui.open_folder("build");
        g.u2_open = cap(format!("Ordner build: Ergebnis {}", r), 64);
    }
    ui.text(TS_BODY | TX_SECONDARY, &g.u2_open);
    ui.box_end();
    // stretched over the page: the line count follows the window width
    ui.text(TS_BODY | TX_WRAP, S_U2_PAGE);
}

ui::main!(ui::launch(&DESC, Gal::default(), &style::STYLE));
