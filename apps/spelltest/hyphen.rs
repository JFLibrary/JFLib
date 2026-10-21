// hyphen -- spelltest --hyphen: hyphenation in wrapped texts and text boxes (ui/hyph.rs) for
// tests/hyphen.uis.  Headless runs read app://hyphen/hyph_de_TEST.dic (written for the
// tests); the UI language (de-DE headless) picks it.  Four texts of the same width: with
// TX_HYPHENATE, without it, with TX_HYPHENATE in English (next_hyph_language: no dictionary,
// so no hyphenation), with soft hyphens (no flag needed); a text box with TB_HYPHENATE; the
// status line names the patterns in use.

use alloc::format;
use alloc::string::String;

use ui::*;

pub const ID_BOX: u32 = 201;

pub const DESC: AppDesc = AppDesc { title: "jfgui Silbentrennung", flags: AF_RESIZABLE, w: 520, h: 640, min_w: 300, min_h: 300, ..AppDesc::NONE };

const BODY: &str = "Die Donaudampfschifffahrtsgesellschaftskapitäne lesen Wörterbücher zur Rechtschreibung.";
/// soft hyphens (U+00AD) only
const SOFT: &str = "Weich: Die Do\u{AD}nau\u{AD}dampf\u{AD}schiff\u{AD}fahrts\u{AD}ge\u{AD}sell\u{AD}schafts\u{AD}ka\u{AD}pi\u{AD}tä\u{AD}ne lesen.";

pub struct Hy {
    text: String,
}

impl Default for Hy {
    fn default() -> Hy {
        Hy { text: String::from(BODY) }
    }
}

impl App for Hy {
    fn init(&mut self, ui: &mut Ui) -> bool {
        if let Some(l) = plat::arg_value("--hyph-lang") {
            ui.hyph_language(&l);
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "Silbentrennung");
        let w = 210;
        ui.next_size(w, 0);
        ui.text(TS_BODY | TX_WRAP | TX_HYPHENATE, &format!("Mit: {}", BODY));
        ui.next_size(w, 0);
        ui.text(TS_BODY | TX_WRAP, &format!("Ohne: {}", BODY));
        ui.next_hyph_language("en");
        ui.next_size(w, 0);
        ui.text(TS_BODY | TX_WRAP | TX_HYPHENATE, &format!("Englisch: {}", BODY));
        ui.next_size(w, 0);
        ui.text(TS_BODY | TX_WRAP, SOFT);
        ui.next_size(w, 132);
        ui.textbox(ID_BOX, None, &mut self.text, 1024 | TB_MULTI | TB_WRAP | TB_HYPHENATE);
        let status = match ui.hyphenator("") {
            Some(h) => format!("Muster: {} + {}", h.patterns().0, h.patterns().1),
            None => String::from("Muster: keine"),
        };
        ui.text(TS_CAPTION, &status);
        ui.box_end();
    }
}
