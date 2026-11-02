// "06 System": what the system tells the app and does for it -- its settings live (scheme,
// contrast, animations, text size, accent; UIC_SETTINGS), a notification (ui.notify; a click
// UIC_NOTE, one that went away UIC_NOTE_GONE), the screen kept on (plat::inhibit with
// INHIBIT_IDLE) and the lifecycle (UIC_ACTIVE, UIC_SUSPEND: 2 in the background, a hidden tab
// or Android's onStop, 0 back).  "Design des Systems" lets the theme follow the system (the
// gallery itself stays light, AF_LIGHT).
// With jfgui_SELFTEST=system (the start intent's extras on Android, debug builds;
// apps/androidGallery/tests/android_system.py) the card scrolls into view and every change is
// written to data://system-state.txt as "key=value" lines.

use alloc::{format, string::String};
use plat::{INHIBIT_IDLE, Inhibit, SysSettings};
use ui::*;
use ui::i18n::{t, tf};

const STATE: &str = "data://system-state.txt";
const ID_SYSTHEME: u32 = 600;
const ID_NOTIFY: u32 = 601;
const ID_KEEPON: u32 = 602;

#[derive(Default)]
pub struct System {
    loaded: bool,
    selftest: bool,
    scrolled: u32,
    scroll_at: u32,
    sys: Option<SysSettings>,
    follow: i32,
    /// UIC_SETTINGS: how many, the last bits; UIC_THEME
    settings: u32,
    bits: u32,
    themes: u32,
    /// UIC_ACTIVE, UIC_SUSPEND (last arg, how often 2 and how often 0 after 2)
    active: bool,
    suspend: u32,
    background: u32,
    foreground: u32,
    /// notifications: the last shown (0: refused at once), clicked, gone; how many tries
    notes: u32,
    shown: u32,
    clicked: u32,
    gone: u32,
    keep: i32,
    held: Option<Inhibit>,
    keep_failed: bool,
    written: String,
}

impl System {
    fn settings(&mut self, ui: &Ui) -> SysSettings {
        // headless script runs: only the jfgui_* overrides (the same everywhere)
        *self.sys.get_or_insert_with(|| if ui.headless() { plat::sys_settings_env() } else { plat::sys_settings() })
    }

    pub fn card(&mut self, ui: &mut Ui) {
        if !self.loaded {
            self.loaded = true;
            self.selftest = plat::env("jfgui_SELFTEST").as_deref() == Some("system");
        }
        let s = self.settings(ui);
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 10);
        ui.heading(2, TS_SUBTITLE, t("06  System"));
        if self.selftest && ui.now() != self.scroll_at {
            // the card at the top of the screen, or as far as the page goes (android_system.py
            // taps its widgets); once a frame, also later: the scroll view has to learn its
            // extent, and a finger on a real phone moves the page.  Down at most 10 times, for a
            // page too short to bring the card up
            self.scroll_at = ui.now();
            let top = ui.px2dip(ui.last_rect()[1]);
            let (_, y) = ui.scroll_pos(10);
            if top < 0 && y > 0 || top > 16 && self.scrolled < 10 {
                self.scrolled += 1;
                ui.scroll_to(10, -1, (y + top - 8).max(0));
            }
        }
        ui.next_size(0, 40);
        let was = self.follow;
        ui.switch(ID_SYSTHEME, t("Design des Systems"), &mut self.follow);
        if self.follow != was {
            ui.theme_set(if self.follow != 0 { 2 } else { 0 });
        }
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        ui.next_size(0, 44);
        if ui.button(ID_NOTIFY, t("Benachrichtigung")) & EV_ACT != 0 {
            self.notes += 1;
            let id = ui.notify(t("JFLib Galerie"), &tf("Benachrichtigung Nummer {}", &[&format!("{}", self.notes)]), None);
            self.shown = id;
        }
        ui.box_end();
        ui.next_size(0, 40);
        let was = self.keep;
        ui.switch(ID_KEEPON, t("Bildschirm anlassen"), &mut self.keep);
        if self.keep != was {
            self.held = if self.keep != 0 { plat::inhibit(t("JFLib Galerie: Bildschirm an"), INHIBIT_IDLE) } else { None };
            self.keep_failed = self.keep != 0 && self.held.is_none();
            if self.keep_failed {
                self.keep = 0;
            }
        }
        let yn = |b: bool| if b { t("ja") } else { t("nein") };
        let sys_dark = match s.dark {
            Some(true) => t("dunkel"),
            Some(false) => t("hell"),
            None => t("keine Angabe"),
        };
        ui.text(TS_BODY | TX_WRAP, &tf("Design: {} (System: {})", &[if ui.theme_dark() { t("dunkel") } else { t("hell") }, sys_dark]));
        let hc = if !ui.high_contrast() {
            t("normal")
        } else if s.contrast_colors.is_some() {
            t("hoch (Systemfarben)")
        } else {
            t("hoch")
        };
        ui.text(TS_BODY | TX_WRAP, &tf("Kontrast: {}  Bewegung: {}", &[hc, if ui.anim_off() { t("reduziert") } else { t("normal") }]));
        let accent = s.accent.map_or(String::from(t("keiner")), |a| format!("#{a:06X}"));
        ui.text(TS_BODY | TX_WRAP, &tf("Text: {} %  Akzent: {}", &[&format!("{}", ui.text_scale()), &accent]));
        ui.text(TS_BODY | TX_WRAP, &tf("Einstellungen: {} Änderungen (zuletzt {}), Design {}", &[&format!("{}", self.settings), &format!("{:#x}", self.bits), &format!("{}", self.themes)]));
        let note = match (self.notes, self.shown) {
            (0, _) => String::from(t("noch keine")),
            (_, 0) => String::from(t("nicht gezeigt")),
            (_, id) if self.clicked == id => format!("#{id} {}", t("geklickt")),
            (_, id) if self.gone == id => format!("#{id} {}", t("weg")),
            (_, id) => format!("#{id} {}", t("gezeigt")),
        };
        ui.text(TS_BODY | TX_WRAP, &tf("Benachrichtigung: {}", &[&note]));
        let on = plat::inhibited() & INHIBIT_IDLE != 0;
        let tail = if self.keep_failed { format!(" ({})", t("nicht möglich")) } else { String::new() };
        ui.text(TS_BODY | TX_WRAP, &tf("Bildschirm an: {}{}", &[yn(on), &tail]));
        ui.text(TS_BODY | TX_WRAP, &tf("Lebenszyklus: aktiv {}, {}, Hintergrund {}×, zurück {}×", &[yn(self.active),
            if self.suspend == 2 { t("im Hintergrund") } else { t("im Vordergrund") }, &format!("{}", self.background), &format!("{}", self.foreground)]));
        ui.box_end();
        self.save(ui);
    }

    /// the library's commands this card follows (false: not one of them)
    pub fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_SETTINGS => {
                self.settings += 1;
                self.bits = arg;
                self.sys = None; // read again
            }
            UIC_THEME => self.themes += 1,
            UIC_ACTIVE => self.active = arg != 0,
            UIC_SUSPEND => {
                if arg == 2 {
                    self.background += 1;
                } else if arg == 0 && self.suspend == 2 {
                    self.foreground += 1;
                }
                self.suspend = arg;
            }
            UIC_NOTE => self.clicked = arg,
            UIC_NOTE_GONE => self.gone = arg,
            _ => return false,
        }
        // the frame may not come (Android: the activity is stopped): the state now
        self.save(ui);
        ui.invalidate();
        true
    }

    // (self-test) data://system-state.txt when it changed
    fn save(&mut self, ui: &Ui) {
        if !self.selftest {
            return;
        }
        let s = self.settings(ui);
        let f = |b: bool| b as u32;
        let text = format!(
            "selftest=system\ndark_system={}\ndark={}\ncontrast={}\ncolors={}\nmotion={}\nscale={}\naccent={}\nsettings={}\nbits={:#x}\n\
             themes={}\nactive={}\nsuspend={}\nbackground={}\nforeground={}\nnotes={}\nshown={}\nclicked={}\ngone={}\nkeep={}\n\
             inhibited={}\n",
            s.dark.map_or("-", |d| if d { "1" } else { "0" }),
            f(ui.theme_dark()),
            f(ui.high_contrast()),
            f(s.contrast_colors.is_some()),
            f(ui.anim_off()),
            ui.text_scale(),
            s.accent.map_or(String::from("-"), |a| format!("{a:06X}")),
            self.settings,
            self.bits,
            self.themes,
            f(self.active),
            self.suspend,
            self.background,
            self.foreground,
            self.notes,
            self.shown,
            self.clicked,
            self.gone,
            self.keep,
            plat::inhibited(),
        );
        if text != self.written {
            plat::write_file(STATE, text.as_bytes());
            self.written = text;
        }
    }
}
