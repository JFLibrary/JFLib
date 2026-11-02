// "Menüs & Auswahl": the popups of the big gallery (apps/gallery/sec_popup.rs): a menu bar with
// submenus and a list of recent files, combo box, date picker, drop-down, split and command
// buttons, a flat tool drop-down with checked rows, tooltips, a context menu (long press, the
// right button) and the same menu from a button.  Commands without shortcuts: they work with the
// finger.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;
use ui::i18n::{list, t, tf};

const ID_COMBO: u32 = 2001;
const ID_DROP: u32 = 2002;
const ID_DROP_ITEM: u32 = 2003; // .. 2005
const ID_DROP_CLOSE: u32 = 2006;
const ID_SPLIT: u32 = 2010;
const ID_SPLIT_TEXT: u32 = 2011;
const ID_SPLIT_IMAGE: u32 = 2012;
const ID_CTX: u32 = 2020;
const ID_CTX_INFO: u32 = 2021;
const ID_POPUP: u32 = 2022;
const ID_TIP: u32 = 2023;
const ID_DATE: u32 = 2024;
const ID_TOOLDROP: u32 = 2030;
const ID_TOOLITEM: u32 = 2031; // .. 2036

const CMD_NEW: u32 = 9001;
const CMD_OPEN: u32 = 9002;
const CMD_SAVE: u32 = 9003;
const CMD_EXIT_PAGE: u32 = 9005;
const CMD_UNDO: u32 = 9010;
const CMD_REDO: u32 = 9011;
const CMD_CUT: u32 = 9012;
const CMD_COPY: u32 = 9013;
const CMD_PASTE: u32 = 9014;
const CMD_GRID: u32 = 9020;
const CMD_ZOOMIN: u32 = 9022;
const CMD_ZOOMOUT: u32 = 9023;
const CMD_ZOOM100: u32 = 9024;
const CMD_LIGHT: u32 = 9025;
const CMD_DARK: u32 = 9026;
const CMD_RECENT0: u32 = 9040; // .. 9047
const CMD_RECENT_CLEAR: u32 = 9050;

pub static CMDS: [Command; 16] = [
    Command::new(CMD_NEW, "&Neu", IC_PAGE, 0, 0, 0),
    Command::new(CMD_OPEN, "Ö&ffnen…", IC_OPEN_FILE, 0, 0, 0),
    Command::new(CMD_SAVE, "&Speichern", IC_SAVE, 0, 0, 0),
    Command::new(CMD_EXIT_PAGE, "Seite &verlassen", 0, 0, 0, 0),
    Command::new(CMD_UNDO, "&Rückgängig", IC_UNDO, 0, 0, CF_DISABLED),
    Command::new(CMD_REDO, "&Wiederholen", IC_REDO, 0, 0, CF_DISABLED),
    Command::new(CMD_CUT, "&Ausschneiden", IC_CUT, 0, 0, 0),
    Command::new(CMD_COPY, "&Kopieren", IC_COPY, 0, 0, 0),
    Command::new(CMD_PASTE, "E&infügen", IC_PASTE, 0, 0, 0),
    Command::new(CMD_GRID, "&Raster", IC_GRID, 0, 0, 0),
    Command::new(CMD_ZOOMIN, "Ver&größern", IC_ZOOM_IN, 0, 0, 0),
    Command::new(CMD_ZOOMOUT, "Ver&kleinern", IC_ZOOM_OUT, 0, 0, 0),
    Command::new(CMD_ZOOM100, "&100 %", 0, 0, 0, 0),
    Command::new(CMD_LIGHT, "&Hell", IC_BRIGHTNESS, 0, 0, 0),
    Command::new(CMD_DARK, "&Dunkel", IC_CONTRAST, 0, 0, 0),
    Command::new(CMD_RECENT_CLEAR, "Liste &leeren", IC_DELETE, 0, 0, 0),
];

static MENUBAR: [Mi; 3] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Bearbeiten", &M_EDIT), Mi::Menu("&Ansicht", &M_VIEW)];
static M_FILE: [Mi; 7] = [
    Mi::Cmd(CMD_NEW),
    Mi::Cmd(CMD_OPEN),
    Mi::Cmd(CMD_SAVE),
    Mi::Sep,
    Mi::Sub("&Zuletzt geöffnet", &M_RECENT),
    Mi::Sep,
    Mi::Cmd(CMD_EXIT_PAGE),
];
static M_EDIT: [Mi; 6] = [Mi::Cmd(CMD_UNDO), Mi::Cmd(CMD_REDO), Mi::Sep, Mi::Cmd(CMD_CUT), Mi::Cmd(CMD_COPY), Mi::Cmd(CMD_PASTE)];
static M_VIEW: [Mi; 4] = [Mi::Cmd(CMD_GRID), Mi::Sep, Mi::Sub("&Zoom", &M_ZOOM), Mi::Sub("&Design", &M_THEME)];
static M_ZOOM: [Mi; 4] = [Mi::Cmd(CMD_ZOOMIN), Mi::Cmd(CMD_ZOOMOUT), Mi::Sep, Mi::Cmd(CMD_ZOOM100)];
static M_THEME: [Mi; 2] = [Mi::Cmd(CMD_LIGHT), Mi::Cmd(CMD_DARK)];
static M_RECENT: [Mi; 3] = [Mi::List(CMD_RECENT0, 8, "Keine Dateien"), Mi::Sep, Mi::Cmd(CMD_RECENT_CLEAR)];
static M_CTX: [Mi; 5] = [Mi::Cmd(CMD_CUT), Mi::Cmd(CMD_COPY), Mi::Cmd(CMD_PASTE), Mi::Sep, Mi::Item(ID_CTX_INFO, "&Eigenschaften", IC_INFO)];

const SHAPES: [&str; 5] = ["Kreis", "Quadrat", "Dreieck", "Stern", "Sechseck"];
const WIDTHS: [&str; 3] = ["Fein", "Mittel", "Breit"]; // i18n
const TOOL_ICONS: [u32; 6] = [IC_PENCIL, IC_BRUSH, IC_ERASER, IC_DROP, IC_EYEDROPPER, IC_ZOOM];
const TOOL_NAMES: [&str; 6] = ["Stift", "Pinsel", "Radierer", "Füllen", "Farbauswahl", "Lupe"]; // i18n

pub struct Menus {
    shape: i32,
    width: i32,
    grid: i32,
    zoom: i32,
    tool: i32,
    date: (i32, u32, u32),
    last: String,
    recent: Vec<String>,
}

impl Default for Menus {
    fn default() -> Self {
        Self {
            shape: 1,
            width: 1,
            grid: 1,
            zoom: 100,
            tool: 1,
            date: (2026, 11, 1),
            last: String::from("–"),
            recent: alloc::vec![String::from(t("urlaub.png")), String::from(t("katze.jpg"))],
        }
    }
}

impl Menus {
    /// command states, then the menu bar, above the scrolling page
    pub fn menubar(&mut self, ui: &mut Ui) {
        let dark = ui.theme_dark();
        ui.cmd_state(CMD_GRID, if self.grid != 0 { CF_CHECKED } else { 0 });
        ui.cmd_state(CMD_LIGHT, if dark { 0 } else { CF_CHECKED });
        ui.cmd_state(CMD_DARK, if dark { CF_CHECKED } else { 0 });
        ui.cmd_state(CMD_ZOOM100, if self.zoom == 100 { CF_DISABLED } else { 0 });
        ui.menu_list(CMD_RECENT0, &self.recent);
        ui.menubar(Some(i18n::menu(&MENUBAR)));
    }

    pub fn page(&mut self, ui: &mut Ui) {
        ui.text(TS_BODY_STRONG, t("Auswahlliste und Kalender"));
        ui.next_size(SIZE_FILL, 44);
        ui.combobox(ID_COMBO, &mut self.shape, &list(&SHAPES));
        let (mut y, mut m, mut d) = self.date;
        ui.next_size(SIZE_FILL, 44);
        if ui.date_picker(ID_DATE, &mut y, &mut m, &mut d) & EV_ACT != 0 {
            self.last = format!("{d}.{m}.{y}");
        }
        self.date = (y, m, d);

        ui.text(TS_BODY_STRONG, t("Aufklappen"));
        ui.next_size(SIZE_FILL, 44);
        ui.dropdown_button(ID_DROP, t("Stärke"));
        if ui.flyout_begin(ID_DROP, FL_BELOW | FL_MINW) {
            for i in 0..3 {
                if self.width == i {
                    ui.next_flags(ST_CHECKED);
                }
                ui.next_size(SIZE_FILL, 44);
                if ui.menu_item(ID_DROP_ITEM + i as u32, t(WIDTHS[i as usize])) & EV_ACT != 0 {
                    self.width = i;
                }
            }
            ui.menu_sep();
            ui.next_size(SIZE_FILL, 44);
            if ui.button(ID_DROP_CLOSE, t("Schließen")) & EV_ACT != 0 {
                ui.popup_close();
            }
        }
        ui.flyout_end();
        ui.next_size(SIZE_FILL, 44);
        ui.next_icon(IC_PASTE);
        if ui.split_button(ID_SPLIT, t("Einfügen")) & EV_ACT != 0 {
            ui.cmd_run(CMD_PASTE, 0);
        }
        if ui.flyout_begin(ID_SPLIT, FL_BELOW | FL_MINW) {
            ui.next_size(SIZE_FILL, 44);
            ui.menu_cmd(CMD_PASTE);
            ui.next_size(SIZE_FILL, 44);
            if ui.menu_item(ID_SPLIT_TEXT, t("Als Text einfügen")) & EV_ACT != 0 {
                self.last = String::from(t("Als Text einfügen"));
            }
            ui.next_size(SIZE_FILL, 44);
            ui.next_icon(IC_PHOTO);
            if ui.menu_item(ID_SPLIT_IMAGE, t("Als Bild einfügen")) & EV_ACT != 0 {
                self.last = String::from(t("Als Bild einfügen"));
            }
        }
        ui.flyout_end();

        ui.text(TS_BODY_STRONG, t("Befehle"));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(48, 44);
        ui.cmd_button(CMD_SAVE, ST_SUBTLE | ST_ICON_ONLY);
        ui.next_size(48, 44);
        ui.cmd_button(CMD_GRID, ST_SUBTLE | ST_ICON_ONLY);
        ui.next_size(SIZE_FILL, 44);
        ui.cmd_button(CMD_UNDO, 0);
        ui.box_end();

        ui.text(TS_BODY_STRONG, t("Kontextmenü"));
        ui.next_size(SIZE_FILL, 44);
        ui.button(ID_CTX, t("Hier lange drücken"));
        ui.context_menu(i18n::menu(&M_CTX));
        ui.next_size(SIZE_FILL, 44);
        if ui.button(ID_POPUP, t("Menü an dieser Stelle")) & EV_ACT != 0 {
            ui.menu_popup(i18n::menu(&M_CTX));
        }
        ui.next_size(SIZE_FILL, 44);
        ui.button(ID_TIP, t("Mit Hinweis (Maus oder Stift)"));
        ui.tooltip(t("Hinweise erscheinen, wenn der Zeiger über dem Element bleibt"));

        ui.text(TS_BODY_STRONG, t("Werkzeug"));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(64, 44);
        ui.next_icon(TOOL_ICONS[self.tool as usize]);
        ui.next_flags(ST_SUBTLE | ST_ICON_ONLY);
        ui.dropdown_button(ID_TOOLDROP, t(TOOL_NAMES[self.tool as usize]));
        if ui.flyout_begin(ID_TOOLDROP, FL_BELOW) {
            for i in 0..6 {
                if self.tool == i as i32 {
                    ui.next_flags(ST_CHECKED);
                }
                ui.next_size(200, 44);
                ui.next_icon(TOOL_ICONS[i]);
                if ui.menu_item(ID_TOOLITEM + i as u32, t(TOOL_NAMES[i])) & EV_ACT != 0 {
                    self.tool = i as i32;
                }
            }
        }
        ui.flyout_end();
        ui.text(TS_BODY, t(TOOL_NAMES[self.tool as usize]));
        ui.box_end();

        ui.text(
            TS_BODY | TX_WRAP | TX_SECONDARY,
            &tf(
                "Letzter Befehl: {}   Form: {}   Stärke: {}   Zoom: {} %",
                &[&self.last, t(SHAPES[self.shape as usize]), t(WIDTHS[self.width as usize]), &format!("{}", self.zoom)],
            ),
        );
    }

    /// the commands of the table and the rows of the menus -> handled; goes_back: "Seite verlassen"
    pub fn command(&mut self, ui: &mut Ui, id: u32, goes_back: &mut bool) -> bool {
        if id == ID_CTX_INFO {
            self.last = String::from(t("Eigenschaften"));
            return true;
        }
        if (CMD_RECENT0..CMD_RECENT0 + 8).contains(&id) {
            if let Some(name) = self.recent.get((id - CMD_RECENT0) as usize) {
                self.last = name.clone();
            }
            return true;
        }
        if !CMDS.iter().any(|c| c.id == id) {
            return false;
        }
        self.last = ui.cmd_label(id);
        match id {
            CMD_EXIT_PAGE => *goes_back = true,
            CMD_GRID => self.grid ^= 1,
            CMD_ZOOMIN => self.zoom += 10,
            CMD_ZOOMOUT => self.zoom = (self.zoom - 10).max(20),
            CMD_ZOOM100 => self.zoom = 100,
            CMD_LIGHT => ui.theme_set(0),
            CMD_DARK => ui.theme_set(1),
            CMD_RECENT_CLEAR => self.recent.clear(),
            _ => {}
        }
        true
    }
}
