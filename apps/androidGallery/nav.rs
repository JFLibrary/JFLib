// The pages behind the start page: what they are, the card and the menu that lead to them, the bar
// above a page.  The back key is Esc for the app while a page is open (plat::back_trap, main.rs).

use alloc::string::String;
use ui::*;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Page {
    #[default]
    Home,
    Buttons,
    Menus,
    Texts,
    Lists,
    Boxes,
    Colors,
    Windows,
    Edge,
    Level,
    Attitude,
    Compass,
    Gps,
}

pub struct PageInfo {
    pub page: Page,
    pub title: &'static str,
    pub icon: u32,
    /// the name for jfgui_PAGE (self-tests)
    pub name: &'static str,
}

const fn info(page: Page, title: &'static str, icon: u32, name: &'static str) -> PageInfo {
    PageInfo { page, title, icon, name }
}

pub const PAGES: [PageInfo; 12] = [
    info(Page::Buttons, "Schaltflächen", IC_POINTER, "buttons"),
    info(Page::Menus, "Menüs & Auswahl", IC_BULLETED_LIST, "menus"),
    info(Page::Texts, "Texteingabe", IC_TEXT_BOX, "texts"),
    info(Page::Lists, "Listen & Bäume", IC_FOLDER, "lists"),
    info(Page::Boxes, "Container", IC_VIEW_ALL, "boxes"),
    info(Page::Colors, "Werte & Farben", IC_COLOR, "colors"),
    info(Page::Windows, "Fenster & Dialoge", IC_OPEN_IN_NEW_WINDOW, "windows"),
    info(Page::Edge, "Randfälle", IC_WARNING, "edge"),
    info(Page::Level, "Wasserwaage", IC_CIRCLE_RING, "level"),
    info(Page::Attitude, "Handylage", IC_ROTATE, "attitude"),
    info(Page::Compass, "Kompass", IC_GLOBE, "compass"),
    info(Page::Gps, "GPS", IC_HOME, "gps"),
];

const FIRST_SENSOR_PAGE: usize = 8;
const ID_MENU: u32 = 900;
const ID_MENU_ROW: u32 = 910; // .. 921
const ID_BACK: u32 = 930;
const ID_CARD_ROW: u32 = 940; // .. 951

pub fn title(page: Page) -> &'static str {
    PAGES.iter().find(|p| p.page == page).map_or("", |p| p.title)
}

/// jfgui_PAGE: the page a self-test starts on
pub fn from_env() -> Page {
    let name: String = plat::env("jfgui_PAGE").unwrap_or_default();
    PAGES.iter().find(|p| p.name == name).map_or(Page::Home, |p| p.page)
}

/// a big row that opens a page
fn row(ui: &mut Ui, id: u32, p: &PageInfo) -> bool {
    ui.next_size(SIZE_FILL, 48);
    ui.next_icon(p.icon);
    ui.next_flags(ST_SUBTLE);
    ui.button(id, p.title) & EV_ACT != 0
}

/// "07 Seiten": the pages as big rows at the end of the start page
pub fn card(ui: &mut Ui) -> Option<Page> {
    let mut to = None;
    ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 4);
    ui.heading(2, TS_SUBTITLE, "07  Weitere Seiten");
    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Alle Widgets der großen Galerie, für den Finger angeordnet.");
    for (k, p) in PAGES.iter().enumerate() {
        if k == FIRST_SENSOR_PAGE {
            ui.separator();
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, "Sensoren und Standort des Geräts.");
        }
        if row(ui, ID_CARD_ROW + k as u32, p) {
            to = Some(p.page);
        }
    }
    ui.box_end();
    to
}

/// the menu of the pages: top right on the start page (no Tab stop: the card is the keyboard's
/// way) and on every page
pub fn menu(ui: &mut Ui, now: Page, focusable: bool) -> Option<Page> {
    let mut to = None;
    ui.next_size(52, 44);
    ui.next_icon(IC_MENU);
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | if focusable { 0 } else { ST_NOFOCUS });
    ui.dropdown_button(ID_MENU, "Seiten");
    if ui.flyout_begin(ID_MENU, FL_BELOW | FL_ALIGN_END) {
        if now != Page::Home {
            ui.next_size(240, 44);
            ui.next_icon(IC_HOME);
            if ui.menu_item(ID_MENU_ROW + PAGES.len() as u32, "Startseite") & EV_ACT != 0 {
                to = Some(Page::Home);
            }
            ui.menu_sep();
        }
        for (k, p) in PAGES.iter().enumerate() {
            ui.next_size(240, 44);
            ui.next_icon(p.icon);
            if p.page == now {
                ui.next_flags(ST_CHECKED);
            }
            // '&' marks a mnemonic in a menu
            if ui.menu_item(ID_MENU_ROW + k as u32, &p.title.replace('&', "&&")) & EV_ACT != 0 {
                to = Some(p.page);
            }
        }
    }
    ui.flyout_end();
    to
}

/// the bar above a page: back, the title, the pages
pub fn top_bar(ui: &mut Ui, page: Page) -> Option<Page> {
    let mut to = None;
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(4), 4);
    ui.next_size(48, 44);
    ui.next_flags(ST_SUBTLE);
    if ui.icon_button(ID_BACK, IC_BACK, "Zurück") & EV_ACT != 0 {
        to = Some(Page::Home);
    }
    ui.next_size(SIZE_FILL, 0);
    ui.heading(1, TS_SUBTITLE | TX_ELLIPSIS, title(page));
    if let Some(p) = menu(ui, page, true) {
        to = Some(p);
    }
    ui.box_end();
    to
}
