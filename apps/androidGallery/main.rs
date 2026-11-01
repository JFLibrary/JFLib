// A compact, touch-friendly gallery using the shared widgets and software renderer.
// New Android example; it also runs on desktop for deterministic UI script checks.
//
// main.rs: the start page (the cards 01 to 07) and the way between the pages (nav.rs).  The cards
// files.rs, secrets.rs, system.rs; behind them what the big gallery (apps/gallery) has, for the
// finger: pg_basic.rs, pg_menus.rs, pg_text.rs, pg_lists.rs, pg_boxes.rs, pg_colors.rs,
// pg_windows.rs, pg_edge.rs; the phone's own: pg_sensors.rs (Wasserwaage, Handylage, Kompass, GPS).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::{format, string::String};
use ui::*;

mod files;
mod nav;
mod pg_basic;
mod pg_boxes;
mod pg_colors;
mod pg_edge;
mod pg_lists;
mod pg_menus;
mod pg_sensors;
mod pg_text;
mod pg_windows;
mod secrets;
mod system;

use nav::Page;

const DESC: AppDesc = AppDesc {
    title: "jfgui Pocket Gallery", w: 390, h: 780, min_w: 280, min_h: 240,
    flags: AF_RESIZABLE | AF_LIGHT, cmds: &pg_menus::CMDS, ..AppDesc::NONE
};
const ID_HOME: u32 = 10; // the start page's scroll view
const ID_PAGE: u32 = 20; // a page's scroll view: + the page's number
const RANGE: Range = Range::new(0, 100, 1, 0);
const COLORS: [u32; 5] = [0xFF0078D4, 0xFF8764B8, 0xFF008575, 0xFFD83B01, 0xFFC239B3];

struct Gallery {
    clicks: u32, enabled: i32, checked: i32, value: i32, color: usize, name: String,
    viewport_height: i32, files: files::Files, secrets: secrets::Secrets, system: system::System,
    page: Page, next: Option<Page>, started: bool, home_y: i32, home_restore: u32,
    basic: pg_basic::Basic, menus: pg_menus::Menus, texts: pg_text::Texts, lists: pg_lists::Lists,
    boxes: pg_boxes::Boxes, colors: pg_colors::Colors, windows: pg_windows::Windows, edge: pg_edge::Edge,
    sensors: pg_sensors::Sensors,
}
impl Default for Gallery {
    fn default() -> Self {
        Self { clicks: 0, enabled: 1, checked: 0, value: 40, color: 0, name: String::new(), viewport_height: 0,
            files: files::Files::default(), secrets: secrets::Secrets::default(), system: system::System::default(),
            page: Page::Home, next: None, started: false, home_y: 0, home_restore: 0,
            basic: Default::default(), menus: Default::default(), texts: Default::default(), lists: Default::default(),
            boxes: Default::default(), colors: Default::default(), windows: Default::default(), edge: Default::default(),
            sensors: Default::default() }
    }
}
impl Gallery {
    // to another page, before this frame builds: the sensors follow, the back key goes back
    fn switch_page(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            let first = nav::from_env(); // (self-tests) jfgui_PAGE
            if first != Page::Home {
                self.next = Some(first);
            }
        }
        let Some(to) = self.next.take() else { return };
        if to == self.page {
            return;
        }
        if self.page == Page::Home {
            self.home_y = ui.scroll_pos(ID_HOME).1;
        } else if to == Page::Home {
            self.home_restore = 2; // the scroll view may have to learn its extent again first
        }
        self.page = to;
        self.sensors.enter(to);
        plat::back_trap(to != Page::Home);
    }

    // a page: the bar, its widgets scrolling below it, what lies above them
    fn subpage(&mut self, ui: &mut Ui, reveal_editor: bool) {
        if let Some(to) = nav::top_bar(ui, self.page) {
            self.next = Some(to);
        }
        if self.page == Page::Menus {
            self.menus.menubar(ui);
        }
        if self.page == Page::Edge {
            self.edge.page(ui);
        } else {
            let scroll = ID_PAGE + self.page as u32;
            ui.next_size(0, SIZE_FILL);
            ui.scroll_begin(scroll, SC_V);
            ui.box_begin(0, BOX_V | PAD_16, 12);
            match self.page {
                Page::Buttons => self.basic.page(ui),
                Page::Menus => self.menus.page(ui),
                Page::Texts => self.texts.page(ui, reveal_editor, scroll),
                Page::Lists => self.lists.page(ui),
                Page::Boxes => self.boxes.page(ui),
                Page::Colors => self.colors.page(ui),
                Page::Windows => self.windows.page(ui),
                Page::Level => self.sensors.level(ui),
                Page::Attitude => self.sensors.attitude(ui),
                Page::Compass => self.sensors.compass(ui),
                Page::Gps => self.sensors.gps(ui),
                Page::Home | Page::Edge => {}
            }
            ui.box_end();
            ui.scroll_end();
        }
        if self.page == Page::Windows {
            self.windows.windows(ui);
        }
    }
}

impl App for Gallery {
    fn frame(&mut self, ui: &mut Ui) {
        self.switch_page(ui);
        let height = ui.client_size().1;
        let reveal_editor = height < self.viewport_height;
        self.viewport_height = height;
        if self.page != Page::Home {
            self.subpage(ui, reveal_editor);
            if self.next.is_some() {
                ui.invalidate();
            }
            return;
        }
        ui.next_size(0, SIZE_FILL);
        ui.scroll_begin(ID_HOME, SC_V);
        ui.box_begin(0, BOX_V | PAD_20, 12);
        // the same rows as before, the menu of the pages beside them
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(SIZE_FILL, 0);
        ui.box_begin(0, BOX_V, 12);
        ui.text(TS_CAPTION | TX_SECONDARY, "jfgui  /  POCKET GALLERY");
        ui.text(TS_TITLE, "Kleine Widgets.");
        ui.text(TS_TITLE, "Echte Library.");
        ui.box_end();
        if let Some(to) = nav::menu(ui, Page::Home, false) {
            self.next = Some(to);
        }
        ui.box_end();
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY,
            "Tippen, ziehen, ausprobieren. Dieselben Rust-Widgets auf dem Smartphone.");

        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 10);
        ui.heading(2, TS_SUBTITLE, "01  Interaktion");
        ui.text(TS_BODY, &format!("Klicks: {}", self.clicks));
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(SIZE_FILL, 44);
        if ui.accent_button(100, "Klick mich") & EV_ACT != 0 { self.clicks += 1; }
        ui.next_size(0, 44);
        if ui.button(101, "Reset") & EV_ACT != 0 { self.clicks = 0; self.value = 40; }
        ui.box_end();
        ui.next_size(0, 40);
        ui.checkbox(110, "Option aktivieren", &mut self.checked);
        ui.next_size(0, 40);
        ui.switch(120, "Regler aktiv", &mut self.enabled);
        ui.text(TS_BODY, &format!("Wert: {} %", self.value));
        ui.next_size(SIZE_FILL, 40);
        if self.enabled == 0 { ui.next_flags(ST_DISABLED); }
        ui.slider(130, &mut self.value, &RANGE);
        ui.progress(131, self.value, 100);
        ui.box_end();

        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 10);
        ui.heading(2, TS_SUBTITLE, "02  Farben & Design");
        ui.box_begin(0, BOX_H, 8);
        for (i, color) in COLORS.iter().enumerate() {
            ui.next_size(40, 40);
            if ui.swatch(200 + i as u32, *color, self.color == i) & EV_ACT != 0 {
                self.color = i;
                ui.accent_set(*color & 0x00FFFFFF);
            }
        }
        ui.box_end();
        ui.next_size(SIZE_FILL, 44);
        if ui.button(210, if ui.theme_dark() { "Helles Design" } else { "Dunkles Design" }) & EV_ACT != 0 {
            ui.theme_set(if ui.theme_dark() { 0 } else { 1 });
        }
        ui.box_end();

        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 10);
        ui.heading(2, TS_SUBTITLE, "03  Texteingabe");
        ui.next_size(SIZE_FILL, 44);
        ui.textbox(300, Some("Dein Name"), &mut self.name, 128);
        // Android's IME reduces the content viewport. Keep the focused editor above it;
        // the same behavior is useful when a desktop preview is resized vertically.
        if reveal_editor && ui.focused() == 300 {
            let rect = ui.last_rect();
            let delta = ui.px2dip(rect[1] + rect[3]) + 16 - height;
            if delta > 0 {
                let (_, y) = ui.scroll_pos(ID_HOME);
                ui.scroll_to(ID_HOME, -1, y + delta);
            }
        }
        ui.text(TS_BODY | TX_WRAP, &if self.name.is_empty() {
            String::from("Hallo Android!")
        } else { format!("Hallo {}!", self.name) });
        ui.box_end();

        self.files.card(ui);
        self.secrets.card(ui);
        self.system.card(ui);
        if let Some(to) = nav::card(ui) {
            self.next = Some(to);
        }
        let platform = if cfg!(target_arch = "aarch64") { "ARM64 / arm64-v8a" }
            else if cfg!(target_os = "android") { "x86_64 / Emulator" } else { "Desktop-Vorschau" };
        ui.text(TS_CAPTION | TX_SECONDARY, platform);
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, "Nativer Rust-Code · eigener Renderer · nach oben wischen");
        ui.box_end();
        ui.scroll_end();
        if self.home_restore > 0 {
            self.home_restore -= 1;
            if ui.scroll_pos(ID_HOME).1 != self.home_y {
                ui.scroll_to(ID_HOME, -1, self.home_y);
            }
        }
        if self.next.is_some() {
            ui.invalidate();
        }
    }
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        // a reading, a fix, the age's timer (pg_sensors.rs)
        if self.sensors.command(ui, id, arg) {
            return true;
        }
        // Esc and the back key (plat::back_trap): a page goes back when nothing took it
        if id == UIC_ESCAPE && self.page != Page::Home {
            self.next = Some(Page::Home);
            ui.invalidate();
            return true;
        }
        let mut back = false;
        if self.menus.command(ui, id, &mut back) {
            if back {
                self.next = Some(Page::Home);
            }
            ui.invalidate();
            return true;
        }
        if self.colors.command(id) || self.edge.command(id) {
            return true;
        }
        // a file of the recent list the browser asked about after a reload (ui.recent_open)
        if id == UIC_RECENT {
            self.files.reopened(ui, arg == 1);
            return true;
        }
        // the system's settings, notifications, the lifecycle (system.rs)
        self.system.command(ui, id, arg)
    }
}
ui::main!(ui::launch(&DESC, Gallery::default(), &style::STYLE));
