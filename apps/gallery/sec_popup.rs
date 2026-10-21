// sec_popup -- the gallery's menu bar, "Menüs & Auswahl", tool row and command handling
//   [apps/gallery/src/main.s: gal_menubar_top, gal_menus, gal_toolrow, gal_menu_cmd,
//    gal_u1_cmd (its popup parts), gal_cmds, gal_menubar, pm_*]
//
// The command table (CMDS) and the menu bar (MENUBAR) are the AppDesc's (main.rs DESC).
// The rows of the "Zuletzt geöffnet" list (MI_LIST, the asm's g_recent buffer) are handed
// to the UI with menu_list before the menu bar is built.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::{cap, Gal, TOOL_ICONS, TOOL_NAMES};

const ID_COMBO: u32 = 8001;
const ID_DROP: u32 = 8002;
const ID_DROP_ITEM: u32 = 8003; // 8003..8005
const ID_DROP_CLOSE: u32 = 8006;
const ID_SPLIT: u32 = 8010; // chevron part: 8010 | 0x80000000
const ID_SPLIT_TEXT: u32 = 8011;
const ID_SPLIT_IMAGE: u32 = 8012;
const ID_CTX: u32 = 8020;
const ID_CTX_INFO: u32 = 8021;
const CMD_NEW: u32 = 9001;
const CMD_OPEN: u32 = 9002;
const CMD_SAVE: u32 = 9003;
const CMD_SAVEAS: u32 = 9004;
const CMD_EXIT: u32 = 9005;
const CMD_UNDO: u32 = 9010;
const CMD_REDO: u32 = 9011;
const CMD_CUT: u32 = 9012;
const CMD_COPY: u32 = 9013;
const CMD_PASTE: u32 = 9014;
const CMD_SELALL: u32 = 9015;
const CMD_GRID: u32 = 9020;
const CMD_STATUS: u32 = 9021;
const CMD_ZOOMIN: u32 = 9022;
const CMD_ZOOMOUT: u32 = 9023;
const CMD_ZOOM100: u32 = 9024;
const CMD_LIGHT: u32 = 9025;
const CMD_DARK: u32 = 9026;
const CMD_LAST: u32 = 9030;
// (U1) lists, key names, command states, tool row
const CMD_RECENT0: u32 = 9040; // .. 9047
const CMD_RECENT_CLEAR: u32 = 9050;
const CMD_TOOLS_F4: u32 = 9051; // F4 accelerator (combo boxes leave F4 alone)
const CMD_RECENT_ADD: u32 = 9052; // test hook: add a recent file
const CMD_BR_L: u32 = 9054; // accelerators on [ ] Strg+Num+ Umschalt+Num5 (script key names)
const CMD_BR_R: u32 = 9055;
const CMD_NUMADD: u32 = 9056;
const CMD_NUM5: u32 = 9057;
const CMD_CST_1000: u32 = 9058; // test hook: 1000 command states set and read back
const CMD_CST_MORE: u32 = 9059; // test hook: 600 more states (over the limit: one UIWARN)
const ID_TOOLDROP: u32 = 7980; // flat drop-down button (tool chooser)
const ID_TOOLITEM: u32 = 7981; // its rows 7981 .. 7986

/// gal_cmds: the command table of the app record
pub(crate) static CMDS: [Command; 24] = [
    Command::new(CMD_NEW, "&Neu", IC_PAGE, MOD_CTRL, b'N' as u32, 0),
    Command::new(CMD_OPEN, "Ö&ffnen…", IC_OPEN_FILE, MOD_CTRL, b'O' as u32, 0),
    Command::new(CMD_SAVE, "&Speichern", IC_SAVE, MOD_CTRL, b'S' as u32, 0),
    Command::new(CMD_SAVEAS, "Speichern &unter…", IC_SAVE_AS, MOD_CTRL | MOD_SHIFT, b'S' as u32, 0),
    Command::new(CMD_EXIT, "&Beenden", 0, 0, 0, 0),
    Command::new(CMD_UNDO, "&Rückgängig", IC_UNDO, MOD_CTRL, b'Z' as u32, CF_DISABLED),
    Command::new(CMD_REDO, "&Wiederholen", IC_REDO, MOD_CTRL, b'Y' as u32, CF_DISABLED),
    Command::new(CMD_CUT, "&Ausschneiden", IC_CUT, MOD_CTRL, b'X' as u32, 0),
    Command::new(CMD_COPY, "&Kopieren", IC_COPY, MOD_CTRL, b'C' as u32, 0),
    Command::new(CMD_PASTE, "E&infügen", IC_PASTE, MOD_CTRL, b'V' as u32, 0),
    Command::new(CMD_SELALL, "Alles aus&wählen", IC_SELECT_ALL, MOD_CTRL, b'A' as u32, 0),
    Command::new(CMD_GRID, "&Raster", IC_GRID, MOD_CTRL, b'G' as u32, 0),
    Command::new(CMD_STATUS, "&Statusleiste", 0, 0, 0, 0),
    Command::new(CMD_ZOOMIN, "Ver&größern", IC_ZOOM_IN, MOD_CTRL, VK_OEM_PLUS, 0),
    Command::new(CMD_ZOOMOUT, "Ver&kleinern", IC_ZOOM_OUT, MOD_CTRL, VK_OEM_MINUS, 0),
    Command::new(CMD_ZOOM100, "&100 %", 0, MOD_CTRL, b'0' as u32, 0),
    Command::new(CMD_LIGHT, "&Hell", IC_BRIGHTNESS, 0, 0, 0),
    Command::new(CMD_DARK, "&Dunkel", IC_CONTRAST, 0, 0, 0),
    Command::new(CMD_RECENT_CLEAR, "Liste &leeren", IC_DELETE, MOD_CTRL | MOD_SHIFT | MOD_ALT, b'L' as u32, 0),
    Command::new(CMD_TOOLS_F4, "Eigenschaften", IC_INFO, 0, VK_F4, 0),
    Command::new(CMD_BR_L, "Kleiner", 0, 0, VK_OEM_4, 0),
    Command::new(CMD_BR_R, "Größer", 0, 0, VK_OEM_6, 0),
    Command::new(CMD_NUMADD, "Plus (Ziffernblock)", 0, MOD_CTRL, VK_ADD, 0),
    Command::new(CMD_NUM5, "Fünf (Ziffernblock)", 0, MOD_SHIFT, VK_NUMPAD0 + 5, 0),
];

/// gal_menubar: the menu bar of the app record
pub(crate) static MENUBAR: [Mi; 3] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Bearbeiten", &M_EDIT), Mi::Menu("&Ansicht", &M_VIEW)];
static M_FILE: [Mi; 8] = [
    Mi::Cmd(CMD_NEW),
    Mi::Cmd(CMD_OPEN),
    Mi::Cmd(CMD_SAVE),
    Mi::Cmd(CMD_SAVEAS),
    Mi::Sep,
    Mi::Cmd(CMD_EXIT),
    Mi::Sep,
    Mi::Sub("&Zuletzt geöffnet", &M_RECENT), // (U1) MI_LIST
];
static M_EDIT: [Mi; 8] = [
    Mi::Cmd(CMD_UNDO),
    Mi::Cmd(CMD_REDO),
    Mi::Sep,
    Mi::Cmd(CMD_CUT),
    Mi::Cmd(CMD_COPY),
    Mi::Cmd(CMD_PASTE),
    Mi::Sep,
    Mi::Cmd(CMD_SELALL),
];
static M_VIEW: [Mi; 5] = [Mi::Cmd(CMD_GRID), Mi::Cmd(CMD_STATUS), Mi::Sep, Mi::Sub("&Zoom", &M_ZOOM), Mi::Sub("&Design", &M_THEME)];
static M_ZOOM: [Mi; 4] = [Mi::Cmd(CMD_ZOOMIN), Mi::Cmd(CMD_ZOOMOUT), Mi::Sep, Mi::Cmd(CMD_ZOOM100)];
static M_THEME: [Mi; 2] = [Mi::Cmd(CMD_LIGHT), Mi::Cmd(CMD_DARK)];
// (U1) rows from S::recent (CMD_RECENT0 + k)
static M_RECENT: [Mi; 3] = [Mi::List(CMD_RECENT0, 8, "Keine Dateien"), Mi::Sep, Mi::Cmd(CMD_RECENT_CLEAR)];
pub(crate) static M_CTX: [Mi; 5] = [Mi::Cmd(CMD_CUT), Mi::Cmd(CMD_COPY), Mi::Cmd(CMD_PASTE), Mi::Sep, Mi::Item(ID_CTX_INFO, "&Eigenschaften", IC_INFO)];
const PM_SHAPES: [&str; 5] = ["Kreis", "Quadrat", "Dreieck", "Stern", "Sechseck"];
const PM_WIDTH_NAMES: [&str; 3] = ["Fein", "Mittel", "Breit"];

/// the section's state (g_pm_*, g_recent*, g_cst_ok, g_f4_count)
pub(crate) struct S {
    shape: i32,
    width: i32,
    grid: i32,
    sbar: i32,
    zoom: i32,
    last: String,
    /// the recent files (g_recent: the MI_LIST rows) and the names added so far
    recent: Vec<String>,
    recent_n: u32,
    cst_ok: i32,
    f4_count: i32,
}

impl Default for S {
    fn default() -> S {
        S {
            shape: 1,
            width: 1,
            grid: 1,
            sbar: 1,
            zoom: 100,
            last: String::from("–"),
            recent: alloc::vec![String::from("urlaub.png"), String::from("katze.jpg")],
            recent_n: 2,
            cst_ok: 0,
            f4_count: 0,
        }
    }
}

// pm_strlist_at(list, i): string i (the end of the list: "")
fn strlist_at<'a>(list: &[&'a str], i: i32) -> &'a str {
    if i < 0 {
        return "";
    }
    list.get(i as usize).copied().unwrap_or("")
}

// pm_set_last(label): remember the last command's name for the status line
fn set_last(g: &mut Gal, s: &str) {
    g.pop.last = cap(String::from(s), 64);
}

/// gal_menubar_top: command states for this frame, then the menu bar (AD_MENUBAR)
pub(crate) fn menubar_top(g: &mut Gal, ui: &mut Ui) {
    let p = &g.pop;
    ui.cmd_state(CMD_GRID, if p.grid != 0 { CF_CHECKED } else { 0 });
    ui.cmd_state(CMD_STATUS, if p.sbar != 0 { CF_CHECKED } else { 0 });
    let dark = ui.theme_dark();
    ui.cmd_state(CMD_LIGHT, if dark { 0 } else { CF_CHECKED });
    ui.cmd_state(CMD_DARK, if dark { CF_CHECKED } else { 0 });
    ui.cmd_state(CMD_ZOOM100, if p.zoom == 100 { CF_DISABLED } else { 0 });
    ui.menu_list(CMD_RECENT0, &p.recent); // (U1) the rows of m_recent (g_recent)
    ui.menubar(None); // the menu bar of the app record (AD_MENUBAR)
}

/// gal_menus: "Menüs & Auswahl" -- combo box, drop-down and split buttons with flyouts,
/// command buttons with tooltips, a context menu and a status line
pub(crate) fn menus(g: &mut Gal, ui: &mut Ui) {
    ui.text(TS_SUBTITLE, "Menüs & Auswahl");
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    // combo box
    ui.combobox(ID_COMBO, &mut g.pop.shape, &PM_SHAPES);
    // drop-down button with a menu-like flyout
    ui.dropdown_button(ID_DROP, "Stärke");
    if ui.flyout_begin(ID_DROP, FL_BELOW) {
        for i in 0..3 {
            if g.pop.width == i {
                ui.next_flags(ST_CHECKED);
            }
            if ui.menu_item(ID_DROP_ITEM + i as u32, PM_WIDTH_NAMES[i as usize]) & EV_ACT != 0 {
                g.pop.width = i;
            }
        }
        ui.menu_sep();
        if ui.button(ID_DROP_CLOSE, "Schließen") & EV_ACT != 0 {
            ui.popup_close();
        }
    }
    ui.flyout_end();
    // split button: the main part pastes, the chevron offers variants
    ui.next_icon(IC_PASTE);
    if ui.split_button(ID_SPLIT, "Einfügen") & EV_ACT != 0 {
        ui.cmd_run(CMD_PASTE, 0);
    }
    if ui.flyout_begin(ID_SPLIT, FL_BELOW) {
        ui.menu_cmd(CMD_PASTE);
        if ui.menu_item(ID_SPLIT_TEXT, "Als Text einfügen") & EV_ACT != 0 {
            set_last(g, "Als Text einfügen");
        }
        ui.next_icon(IC_PHOTO);
        if ui.menu_item(ID_SPLIT_IMAGE, "Als Bild einfügen") & EV_ACT != 0 {
            set_last(g, "Als Bild einfügen");
        }
    }
    ui.flyout_end();
    // command buttons (icon, tooltip with the accelerator, state from the table)
    ui.cmd_button(CMD_SAVE, ST_SUBTLE | ST_ICON_ONLY);
    ui.cmd_button(CMD_GRID, ST_SUBTLE | ST_ICON_ONLY);
    ui.cmd_button(CMD_UNDO, 0);
    ui.box_end();

    // a context menu on a button, and the status line
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    ui.button(ID_CTX, "Rechtsklick hier");
    ui.context_menu(&M_CTX);
    let p = &g.pop;
    let s = cap(
        format!(
            "Letzter Befehl: {}   Form: {}   Stärke: {}   Zoom: {} %",
            p.last,
            strlist_at(&PM_SHAPES, p.shape),
            PM_WIDTH_NAMES[p.width as usize],
            p.zoom
        ),
        192,
    );
    ui.text(TS_BODY | TX_SECONDARY, &s);
    ui.box_end();
}

/// gal_toolrow: a flat (tool bar) drop-down that shows the current tool's icon; its flyout
/// has checked rows with icons (check column + icon column); command state count
pub(crate) fn toolrow(g: &mut Gal, ui: &mut Ui) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY, "Werkzeug:");
    ui.next_icon(TOOL_ICONS[g.tool as usize]);
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY);
    ui.dropdown_button(ID_TOOLDROP, TOOL_NAMES[g.tool as usize]);
    if ui.flyout_begin(ID_TOOLDROP, FL_BELOW) {
        for i in 0..6 {
            if g.tool == i as i32 {
                ui.next_flags(ST_CHECKED);
            }
            ui.next_icon(TOOL_ICONS[i]);
            if ui.menu_item(ID_TOOLITEM + i as u32, TOOL_NAMES[i]) & EV_ACT != 0 {
                g.tool = i as i32;
            }
        }
    }
    ui.flyout_end();
    let s = cap(format!("Befehlszustände: {}", g.pop.cst_ok), 64);
    ui.text(TS_BODY | TX_SECONDARY, &s);
    ui.box_end();
}

/// gal_menu_cmd(id, arg) -> handled: the menu commands CMD_NEW .. CMD_LAST, ID_CTX_INFO
pub(crate) fn menu_cmd(g: &mut Gal, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if id == ID_CTX_INFO {
        set_last(g, "Eigenschaften");
        return true;
    }
    if !(CMD_NEW..CMD_LAST).contains(&id) {
        return false;
    }
    let l = ui.cmd_label(id);
    set_last(g, &l);
    let p = &mut g.pop;
    match id {
        CMD_GRID => p.grid ^= 1,
        CMD_STATUS => p.sbar ^= 1,
        CMD_ZOOMIN => p.zoom += 10,
        CMD_ZOOMOUT => {
            if p.zoom > 20 {
                p.zoom -= 10;
            }
        }
        CMD_ZOOM100 => p.zoom = 100,
        CMD_LIGHT => ui.theme_set(0),
        CMD_DARK => ui.theme_set(1),
        CMD_EXIT => ui.quit(),
        _ => {}
    }
    true
}

/// gal_u1_cmd(id, arg) -> handled, its popup parts: recent files (CMD_RECENT0 ..,
/// CMD_RECENT_CLEAR, CMD_RECENT_ADD; Gal::recent_last), command states (CMD_CST_1000,
/// CMD_CST_MORE), key names (CMD_BR_L ..), CMD_TOOLS_F4
pub(crate) fn u1_cmd(g: &mut Gal, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if (CMD_RECENT0..CMD_RECENT0 + 8).contains(&id) {
        // a recent file was chosen: remember its name
        let r = &g.pop.recent;
        let k = (id - CMD_RECENT0) as usize;
        let s = if k < r.len() { r[k].clone() } else { String::new() };
        g.recent_last = cap(s, 64);
        return true;
    }
    match id {
        CMD_RECENT_CLEAR => g.pop.recent.clear(),
        CMD_RECENT_ADD => {
            // append "bildN.png" to the list (up to 8 names)
            let p = &mut g.pop;
            if p.recent_n < 8 {
                p.recent_n += 1;
                p.recent.push(cap(format!("bild{}.png", p.recent_n), 32));
            }
        }
        CMD_CST_1000 => {
            // 1000 command states, every third one disabled, read back
            for i in 0..1000u32 {
                ui.cmd_state(i + 20000, if i % 3 == 0 { CF_DISABLED } else { 0 });
            }
            g.pop.cst_ok = 0;
            for i in 0..1000u32 {
                let want = if i % 3 == 0 { CF_DISABLED } else { 0 };
                if ui.cmd_flags(i + 20000) == want {
                    g.pop.cst_ok += 1;
                }
            }
        }
        CMD_CST_MORE => {
            // 600 more: beyond the table limit (1536) one UIWARN
            for i in 0..600u32 {
                ui.cmd_state(i + 30000, CF_CHECKED);
            }
        }
        CMD_BR_L..=CMD_NUM5 => {
            // key name tests: the label as the last command
            let l = ui.cmd_label(id);
            set_last(g, &l);
        }
        CMD_TOOLS_F4 => {
            g.pop.f4_count += 1;
            set_last(g, "Eigenschaften (F4)");
        }
        _ => return false,
    }
    true
}
