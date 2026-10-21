// fontpicktest -- the system's fonts in the UI: generic families and substitutes
// (text::fontconf), loading a choice (ui.font_load), the font picker and the font dialog
// (ui/fontpick.rs), the system's font smoothing and its subpixel order; for tests/picker.uis,
// dialog.uis, aa.uis, subpixel_*.uis
//
//   build/win/fontpicktest.exe --script apps/fontpicktest/tests/picker.uis
//   build/win/fontpicktest.exe        (a window on the system's fonts)
//
// Options (set before the runtime starts): --font-dirs DIRS (jfgui_FONTDIRS: these folders
// instead of the system's), --fontconf FILE (jfgui_FONTCONF: these rules instead of
// fontconfig's), --font-aa 0|1 (jfgui_FONT_AA: the system's font smoothing), --font-rgba
// none|rgb|bgr|vrgb|vbgr (jfgui_FONT_RGBA: its subpixel order; headless runs smooth in grey
// without it).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Schriften", flags: AF_RESIZABLE, w: 720, h: 620, ..AppDesc::NONE };

const PICKER: u32 = wid("picker");
const DIALOG: u32 = wid("dialog");
const GENERICS: [&str; 5] = ["sans-serif", "serif", "monospace", "Helvetica", "Unbekannt"];

struct Fonts {
    sel: FontSel,
    dlg_open: Shared<i32>,
    accepted: u32,
}

impl Fonts {
    fn new() -> Fonts {
        let args = plat::args();
        for (opt, var) in [("--font-dirs", "jfgui_FONTDIRS"), ("--fontconf", "jfgui_FONTCONF"), ("--font-aa", "jfgui_FONT_AA"),
                           ("--font-rgba", "jfgui_FONT_RGBA")] {
            if let Some(k) = args.iter().position(|a| a == opt) {
                if let Some(v) = args.get(k + 1) {
                    plat::setenv(var, Some(v));
                }
            }
        }
        Fonts { sel: FontSel::default(), dlg_open: Shared::new(0), accepted: 0 }
    }
}

fn describe(s: &FontSel) -> String {
    format!("{} {} {} {}", s.family, s.weight, if s.italic { "kursiv" } else { "normal" }, s.size)
}

impl App for Fonts {
    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "Schriften");
        if ui.font_picker(PICKER, &mut self.sel) & EV_ACT != 0 {
            ui.repass();
        }
        ui.text(TS_BODY, &format!("Auswahl: {}", describe(&self.sel)));
        let sel = self.sel.clone();
        let found = ui.font_resolved(&sel);
        ui.text(TS_BODY, &format!("Geladen: {}", found));
        ui.next_size(-1, 64);
        ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_CENTER | pad(12), 0);
        match ui.font_load(&sel) {
            Some(f) => ui.text_in_font(f, sel.size, "Franz jagt im Taxi quer durch Bayern"),
            None => ui.text(TS_BODY, "keine Schrift"),
        }
        ui.box_end();
        if ui.button(ui.id("open"), "Schriftart…") & EV_ACT != 0 {
            self.dlg_open.set(1);
        }
        // the generic families and a substitute on this system's rules
        for g in GENERICS {
            let f = ui.font_resolved(&FontSel::new(g, 11.0));
            ui.text(TS_CAPTION, &format!("{} → {}", g, f));
        }
        let ja: Vec<String> = ui.font_conf().candidates("sans-serif", "ja").into_iter().take(2).collect();
        ui.text(TS_CAPTION, &format!("ja: {}", ja.join(", ")));
        ui.text(TS_CAPTION, &format!("Glättung: {}  Dialog: {}", if ui.text.antialias() { "an" } else { "aus" }, self.accepted));
        let order = ["keine", "RGB", "BGR", "VRGB", "VBGR"].get(ui.text.subpixel() as usize).copied().unwrap_or("?");
        ui.text(TS_CAPTION, &format!("Subpixel: {}", order));
        ui.box_end();
        if ui.font_dialog(DIALOG, &self.dlg_open, &mut self.sel) & EV_ACT != 0 {
            self.accepted += 1;
        }
    }
}

ui::main!(ui::launch(&DESC, Fonts::new(), &style::STYLE));
