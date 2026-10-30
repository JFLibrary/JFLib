// A compact, touch-friendly gallery using the shared widgets and software renderer.
// New Android example; it also runs on desktop for deterministic UI script checks.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::{format, string::String};
use ui::*;

mod files;
mod secrets;

const DESC: AppDesc = AppDesc {
    title: "jfgui Pocket Gallery", w: 390, h: 780, min_w: 280, min_h: 240,
    flags: AF_RESIZABLE | AF_LIGHT, ..AppDesc::NONE
};
const RANGE: Range = Range::new(0, 100, 1, 0);
const COLORS: [u32; 5] = [0xFF0078D4, 0xFF8764B8, 0xFF008575, 0xFFD83B01, 0xFFC239B3];

struct Gallery {
    clicks: u32, enabled: i32, checked: i32, value: i32, color: usize, name: String,
    viewport_height: i32, files: files::Files, secrets: secrets::Secrets,
}
impl Default for Gallery {
    fn default() -> Self {
        Self { clicks: 0, enabled: 1, checked: 0, value: 40, color: 0, name: String::new(), viewport_height: 0,
            files: files::Files::default(), secrets: secrets::Secrets::default() }
    }
}
impl App for Gallery {
    fn frame(&mut self, ui: &mut Ui) {
        let height = ui.client_size().1;
        let reveal_editor = height < self.viewport_height;
        self.viewport_height = height;
        ui.next_size(0, SIZE_FILL);
        ui.scroll_begin(10, SC_V);
        ui.box_begin(0, BOX_V | PAD_20, 12);
        ui.text(TS_CAPTION | TX_SECONDARY, "jfgui  /  POCKET GALLERY");
        ui.text(TS_TITLE, "Kleine Widgets.");
        ui.text(TS_TITLE, "Echte Library.");
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
                let (_, y) = ui.scroll_pos(10);
                ui.scroll_to(10, -1, y + delta);
            }
        }
        ui.text(TS_BODY | TX_WRAP, &if self.name.is_empty() {
            String::from("Hallo Android!")
        } else { format!("Hallo {}!", self.name) });
        ui.box_end();

        self.files.card(ui);
        self.secrets.card(ui);
        let platform = if cfg!(target_arch = "aarch64") { "ARM64 / arm64-v8a" }
            else if cfg!(target_os = "android") { "x86_64 / Emulator" } else { "Desktop-Vorschau" };
        ui.text(TS_CAPTION | TX_SECONDARY, platform);
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, "Nativer Rust-Code · eigener Renderer · nach oben wischen");
        ui.box_end();
        ui.scroll_end();
    }
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        // a file of the recent list the browser asked about after a reload (ui.recent_open)
        if id == UIC_RECENT {
            self.files.reopened(ui, arg == 1);
            return true;
        }
        false
    }
}
ui::main!(ui::launch(&DESC, Gallery::default(), &style::STYLE));
