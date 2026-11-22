// spelltest -- spell checking: text boxes with TB_SPELL (ui/edit_spell.rs) and the spell crate
// (lib/spell), for tests/*.uis and by hand
//
//   build/win/spelltest.exe                      by hand: the system's checker (Windows
//                                                ISpellChecker) or the Hunspell files of the
//                                                system, else the test dictionaries of app://
//   build/win/spelltest.exe --spell-lang en      another language (also the buttons)
//   build/win/spelltest.exe --spell-async        suggestions on a worker in headless runs too
//                                                (tests/async.uis: the menu waits for them)
//   build/win/spelltest.exe --test engine [--only GROUP]   self-checks of the spell crate
//                                                (engine.rs); exit code = failures
//   build/win/spelltest.exe --test hostile [--only NAME]   crafted dictionaries against the
//                                                crate's bounds (hostile.rs); exit code =
//                                                operations over their limit
//   build/win/spelltest.exe --hyphenate DIC WORDS   every line of WORDS hyphenated with a
//                                                hyphenation dictionary from any path
//   build/win/spelltest.exe --hyphen [--hyph-lang TAG]   hyphenated texts and a text box
//                                                (hyphen.rs, tests/hyphen.uis)
//   build/win/spelltest.exe --compare AFF DIC WORDS [--sug N]   every line of WORDS checked with
//                                                a dictionary from any path (compare.rs)
//   build/win/spelltest.exe --script apps/spelltest/tests/menu.uis
//   build/win/spelltest.exe --report --auto-close 4000   a real window that prints its status
//                                                lines when they change (checks window mode:
//                                                the system's checker, loading on a worker)
//
// Headless runs (scripts) use only app://hunspell: de_TEST and en_TEST (assets/hunspell, written
// for these tests).  The window: "Betreff" (one line, TB_SPELL), "Nachricht" (TB_MULTI |
// TB_WRAP | TB_SPELL), "Ohne Prüfung" (no TB_SPELL) and "Kennwort" (TB_PASSWORD | TB_SPELL:
// never checked); the switch "Rechtschreibung prüfen" (spell_enable); the language buttons
// (spell_language); the status lines: language and backend, the misspelled words of each box
// (spell_errors), the personal word list.  Command 1 opens "Zweites Fenster" (#120) with a
// "Betreff" box of the same id (#110) and other text: each window checks its own box and opens
// the menu of its own (tests/windows.uis).  Command 2 closes it, command 3 frees its state
// (window_destroy): a menu open in it goes with it (tests/window_gone.uis).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod compare;
mod engine;
mod hostile;
mod hyphen;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;
use ui::i18n::{t, tf, Catalog};

const ID_ON: u32 = 101;
const ID_DE: u32 = 102;
const ID_EN: u32 = 103;
const ID_USER: u32 = 104;
const ID_SUBJECT: u32 = 110;
const ID_BODY: u32 = 111;
const ID_PLAIN: u32 = 112;
const ID_PW: u32 = 113;
const ID_SECOND: u32 = 120; // command 1: "Zweites Fenster", a "Betreff" box of the same id
const CMD_SECOND: u32 = 1;
const CMD_SECOND_CLOSE: u32 = 2; // the second window closes (hidden)
const CMD_SECOND_DESTROY: u32 = 3; // the second window's state is freed (ui.window_destroy)
const SECOND: &str = "Fehlr und das Haus";

const DESC: AppDesc = AppDesc { title: "JFLib Rechtschreibung", flags: AF_RESIZABLE, w: 640, h: 560, min_w: 360, min_h: 300, ..AppDesc::NONE }; // i18n

const SUBJECT: &str = "Die Hasu-Tür";
const BODY: &str = "Heute ist die Arbit in der Strasse sehr schön. Die Hausaufgaben sind gut, z.B. das Wörterbuch.\nJFLib und E-Mail-Adresse, 3D, https://example.org/wort";

struct Spell {
    /// command 1: the second window (open, placement) and its "Betreff"
    second_open: Shared<i32>,
    second_rect: Shared<WinRect>,
    second: String,
    on: i32,
    report: Option<String>, // --report: the status printed last
    subject: String,
    body: String,
    plain: String,
    pw: String,
}

impl Default for Spell {
    fn default() -> Spell {
        Spell {
            second_open: Shared::new(0),
            second_rect: Shared::new(WinRect::new(40, 40, 420, 200, 0)),
            second: String::from(SECOND),
            on: 1,
            report: None,
            subject: String::from(SUBJECT),
            body: String::from(BODY),
            plain: String::from("Ohne Prüfung: Hasu"),
            pw: String::from("Hasu"),
        }
    }
}

// "Hasu, Arbit" for the misspelled ranges of a text
fn words(text: &str, r: &[(usize, usize)]) -> String {
    let v: Vec<&str> = r.iter().filter_map(|&(a, b)| text.get(a..b)).collect();
    if v.is_empty() { String::from("-") } else { v.join(", ") }
}

fn state_name(s: u32) -> &'static str {
    match s {
        SPELL_LOADING => t("lädt"),
        SPELL_READY => t("bereit"),
        SPELL_NONE => t("kein Wörterbuch"),
        _ => t("aus"),
    }
}

impl App for Spell {
    fn init(&mut self, ui: &mut Ui) -> bool {
        if let Some(l) = plat::arg_value("--spell-lang") {
            ui.spell_language(&l);
        } else if i18n::language_code() != "de" {
            ui.spell_language("de"); // the samples are German, whatever the window's language
        }
        if plat::args().iter().any(|a| a == "--report") {
            self.report = Some(String::new());
        }
        if plat::args().iter().any(|a| a == "--spell-async") {
            ui.spell_suggest_async(true);
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, t("Rechtschreibung"));
        ui.box_begin(0, BOX_H, 8);
        if ui.checkbox(ID_ON, t("Rechtschreibung prüfen"), &mut self.on) & EV_ACT != 0 {
            ui.spell_enable(self.on != 0);
        }
        if ui.button(ID_DE, t("Deutsch")) & EV_ACT != 0 {
            ui.spell_language("de");
        }
        if ui.button(ID_EN, t("English")) & EV_ACT != 0 {
            ui.spell_language("en");
        }
        if ui.button(ID_USER, t("Benutzersprachen")) & EV_ACT != 0 {
            ui.spell_language("");
        }
        ui.box_end();
        ui.text(TS_BODY, t("Betreff"));
        ui.next_size(-1, 0);
        ui.textbox(ID_SUBJECT, Some(t("Betreff")), &mut self.subject, 256 | TB_SPELL);
        ui.text(TS_BODY, t("Nachricht"));
        ui.next_size(-1, 0);
        ui.textbox(ID_BODY, Some(t("Nachricht")), &mut self.body, 4096 | TB_MULTI | TB_WRAP | TB_SPELL);
        ui.text(TS_BODY, t("Ohne Prüfung"));
        ui.next_size(-1, 0);
        ui.textbox(ID_PLAIN, None, &mut self.plain, 256);
        ui.text(TS_BODY, t("Kennwort"));
        ui.next_size(-1, 0);
        ui.textbox(ID_PW, None, &mut self.pw, 64 | TB_PASSWORD | TB_SPELL);
        // the status: checker, misspelled words, the personal word list
        let st = ui.spell_state();
        let (lang, backend, personal) = match ui.speller() {
            Some(s) => (String::from(s.language()), s.backend(), s.personal().join(", ")),
            None => (String::from("-"), "-", String::new()),
        };
        let e1 = ui.spell_errors(ID_SUBJECT);
        let e2 = ui.spell_errors(ID_BODY);
        let e3 = ui.spell_errors(ID_PW);
        let lines = [
            tf("Sprache: {} ({}), {}", &[&lang, backend, state_name(st)]),
            tf("Fehler Betreff: {}", &[&words(&self.subject, &e1)]),
            tf("Fehler Nachricht: {}", &[&words(&self.body, &e2)]),
            tf("Fehler Kennwort: {}", &[&words(&self.pw, &e3)]),
            tf("Wörterbuch: {}", &[if personal.is_empty() { "-" } else { personal.as_str() }]),
        ];
        for l in &lines {
            ui.text(TS_CAPTION, l);
        }
        if let Some(last) = self.report.as_mut() {
            let now = lines.join(" | ");
            if *last != now {
                plat::write_stdout(format!("{}\n", now).as_bytes());
                *last = now;
            }
        }
        ui.box_end();
        // command 1: a box of the same id in another window, checked by itself
        let (open, rect) = (self.second_open.clone(), self.second_rect.clone());
        if ui.window_begin(ID_SECOND, t("Zweites Fenster"), Some(&open), WF_TOOL, Some(&rect)) {
            ui.box_begin(0, BOX_V | pad(12), 8);
            ui.next_size(-1, 0);
            ui.textbox(ID_SUBJECT, Some(t("Betreff")), &mut self.second, 256 | TB_SPELL);
            let e = ui.spell_errors(ID_SUBJECT);
            ui.text(TS_CAPTION, &tf("Fehler Betreff 2: {}", &[&words(&self.second, &e)]));
            ui.box_end();
        }
        ui.window_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_SECOND => self.second_open.set(1),
            CMD_SECOND_CLOSE => self.second_open.set(0),
            CMD_SECOND_DESTROY => {
                self.second_open.set(0);
                ui.window_destroy(ID_SECOND);
            }
            _ => return false,
        }
        true
    }
}

fn spelltest() -> i32 {
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")).with_library_languages());
    if plat::arg_value("--test").as_deref() == Some("engine") {
        return engine::run();
    }
    if plat::arg_value("--test").as_deref() == Some("hostile") {
        return hostile::run();
    }
    if plat::args().iter().any(|a| a == "--compare") {
        return compare::run();
    }
    if plat::args().iter().any(|a| a == "--hyphenate") {
        return compare::hyphenate();
    }
    if plat::args().iter().any(|a| a == "--hyphen") {
        return ui::launch(&hyphen::DESC.translated(), hyphen::Hy::default(), &style::STYLE);
    }
    ui::launch(&DESC.translated(), Spell::default(), &style::STYLE)
}

ui::main!(spelltest());
