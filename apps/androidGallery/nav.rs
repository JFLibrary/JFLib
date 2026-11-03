// The pages behind the start page: what they are, the card and the menu that lead to them, the bar
// above a page.  The back key is Esc for the app while a page is open (plat::back_trap, main.rs).

use alloc::string::String;
use ui::*;
use ui::i18n::t;

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

// i18n: title
pub const PAGES: [PageInfo; 12] = [
    PageInfo { page: Page::Buttons, title: "Schaltflächen", icon: IC_POINTER, name: "buttons" },
    PageInfo { page: Page::Menus, title: "Menüs & Auswahl", icon: IC_BULLETED_LIST, name: "menus" },
    PageInfo { page: Page::Texts, title: "Texteingabe", icon: IC_TEXT_BOX, name: "texts" },
    PageInfo { page: Page::Lists, title: "Listen & Bäume", icon: IC_FOLDER, name: "lists" },
    PageInfo { page: Page::Boxes, title: "Container", icon: IC_VIEW_ALL, name: "boxes" },
    PageInfo { page: Page::Colors, title: "Werte & Farben", icon: IC_COLOR, name: "colors" },
    PageInfo { page: Page::Windows, title: "Fenster & Dialoge", icon: IC_OPEN_IN_NEW_WINDOW, name: "windows" },
    PageInfo { page: Page::Edge, title: "Randfälle", icon: IC_WARNING, name: "edge" },
    PageInfo { page: Page::Level, title: "Wasserwaage", icon: IC_CIRCLE_RING, name: "level" },
    PageInfo { page: Page::Attitude, title: "Handylage", icon: IC_ROTATE, name: "attitude" },
    PageInfo { page: Page::Compass, title: "Kompass", icon: IC_GLOBE, name: "compass" },
    PageInfo { page: Page::Gps, title: "GPS", icon: IC_HOME, name: "gps" },
];

const FIRST_SENSOR_PAGE: usize = 8;
const ID_MENU: u32 = 900;
const ID_MENU_ROW: u32 = 910; // .. 921
const ID_BACK: u32 = 930;
const ID_CARD_ROW: u32 = 940; // .. 951

pub fn title(page: Page) -> &'static str {
    PAGES.iter().find(|p| p.page == page).map_or("", |p| t(p.title))
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
    ui.button(id, t(p.title)) & EV_ACT != 0
}

/// "07 Seiten": the pages as big rows at the end of the start page
pub fn card(ui: &mut Ui) -> Option<Page> {
    let mut to = None;
    ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 4);
    ui.heading(2, TS_SUBTITLE, t("07  Weitere Seiten"));
    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Alle Widgets der großen Galerie, für den Finger angeordnet."));
    for (k, p) in PAGES.iter().enumerate() {
        if k == FIRST_SENSOR_PAGE {
            ui.separator();
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Sensoren und Standort des Geräts."));
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
    ui.dropdown_button(ID_MENU, t("Seiten"));
    if ui.flyout_begin(ID_MENU, FL_BELOW | FL_ALIGN_END) {
        if now != Page::Home {
            ui.next_size(240, 44);
            ui.next_icon(IC_HOME);
            if ui.menu_item(ID_MENU_ROW + PAGES.len() as u32, t("Startseite")) & EV_ACT != 0 {
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
            if ui.menu_item(ID_MENU_ROW + k as u32, &t(p.title).replace('&', "&&")) & EV_ACT != 0 {
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
    if ui.icon_button(ID_BACK, IC_BACK, t("Zurück")) & EV_ACT != 0 {
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
