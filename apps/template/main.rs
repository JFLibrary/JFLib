// template -- minimal app: the starting point for new apps          [apps/template/src/main.s]
//
// Build and run:   python build_rs.py run template
// Screenshot:      build/win/template.exe --shot shot.png --scale 150
// Test script:     build/win/template.exe --script apps/template/tests/smoke.uis
//
// An app implements ui::App (frame builds the UI every pass, command gets the commands
// queued with ui.cmd_run) and passes its descriptor, its state and the linked style to
// ui::run through the ui::main! entry point.  German is the source language; locales/ui.tsv
// has the other languages, i18n::t reads them (docs/GUIDE.md "Languages").
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use ui::*;
use ui::i18n::{t, tf, Catalog};

const ID_HELLO: u32 = 100;
const ID_RESET: u32 = 101;
const CMD_RESET: u32 = 1;

const DESC: AppDesc = AppDesc {
    title: "JFLib Template", // i18n
    flags: AF_RESIZABLE,
    w: 480,
    h: 320,
    min_w: 320,
    min_h: 240,
    ..AppDesc::NONE
};

#[derive(Default)]
struct Template {
    count: i32,
}

impl App for Template {
    // one vertical stack: title, text, two buttons
    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(24), 12);
        ui.text(TS_TITLE, t("Hallo, JFLib!"));
        ui.text(TS_BODY | TX_SECONDARY, &tf("Klicks: {}", &[&alloc::format!("{}", self.count)]));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.accent_button(ID_HELLO, t("Klick mich")) & EV_ACT != 0 {
            self.count += 1;
        }
        if ui.button(ID_RESET, t("Zurücksetzen")) & EV_ACT != 0 {
            ui.cmd_run(CMD_RESET, 0); // commands run after the frame
        }
        ui.box_end();
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id != CMD_RESET {
            return false;
        }
        self.count = 0;
        true
    }
}

ui::main!({
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")).with_library_languages());
    ui::launch(&DESC.translated(), Template::default(), &style::STYLE)
});
