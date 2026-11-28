// "06 System": what the system tells the app and does for it -- its settings live (scheme,
// contrast, animations, text size, accent; UIC_SETTINGS), a notification (ui.notify; a click
// UIC_NOTE, one that went away UIC_NOTE_GONE), the screen kept on (plat::inhibit with
// INHIBIT_IDLE), the share sheet (plat::share: a text, a link and a small file; the answer of
// share_poll shown), printing (plat::print: a test picture on A4, Android's print dialog with
// "Save as PDF"; docs/ANDROID.md AN4) and the lifecycle (UIC_ACTIVE, UIC_SUSPEND: 2 in the background, a hidden tab
// or Android's onStop, 0 back).  "Design des Systems" lets the theme follow the system (the
// gallery itself stays light, AF_LIGHT).
// With jfgui_SELFTEST=system (the start intent's extras on Android, debug builds;
// apps/androidGallery/tests/android_system.py) the card scrolls into view and every change is
// written to data://system-state.txt as "key=value" lines.

use alloc::{format, string::String};
use plat::{INHIBIT_IDLE, Inhibit, SHARE_CANCELLED, SHARE_DONE, SHARE_FAILED, SHARE_NONE, SHARE_PENDING, Share, SysSettings};
use ui::*;
use ui::i18n::{t, tf};

const STATE: &str = "data://system-state.txt";
const ID_SYSTHEME: u32 = 600;
const ID_NOTIFY: u32 = 601;
const ID_KEEPON: u32 = 602;
const ID_SHARE: u32 = 603;
const ID_PRINT: u32 = 604;
/// the small file the share button hands over
const SHARE_FILE: &str = "data://Galerie-Teilen.txt";

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
    /// the share sheet: how often opened, the last answer (SHARE_*; SHARE_PENDING while open)
    shares: u32,
    share: i32,
    /// printing: how often, the last answer of plat::print (0: the system's dialog has the job)
    prints: u32,
    print: i32,
    written: String,
}

/// the picture the button prints: four bands (red, green, blue, black) in a grey frame on white,
/// easy to tell apart in the PDF ("Save as PDF")
fn test_picture() -> alloc::vec::Vec<u8> {
    let (w, h) = (PRINT_W as usize, PRINT_H as usize);
    let mut px = alloc::vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let frame = x < 8 || y < 8 || x >= w - 8 || y >= h - 8;
            let bgr: [u8; 3] = if frame {
                [128, 128, 128]
            } else if y < 24 || y >= h - 24 || x < 24 || x >= w - 24 {
                [255, 255, 255]
            } else {
                match (x - 24) * 4 / (w - 48) {
                    0 => [0, 0, 220],
                    1 => [0, 170, 0],
                    2 => [220, 0, 0],
                    _ => [0, 0, 0],
                }
            };
            px[(y * w + x) * 4..][..4].copy_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
        }
    }
    px
}
const PRINT_W: i32 = 480;
const PRINT_H: i32 = 320;

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
        ui.next_size(0, 44);
        if ui.button(ID_SHARE, t("Teilen")) & EV_ACT != 0 {
            self.shares += 1;
            plat::write_file(SHARE_FILE, "JFLib Galerie: eine Datei zum Teilen".as_bytes());
            let s = Share {
                title: String::from(t("JFLib Galerie")),
                text: String::from(t("Geteilt aus der JFLib Galerie")),
                url: String::from("https://jflib.org/"),
                files: alloc::vec![String::from(SHARE_FILE)],
                anchor: ui.last_rect(),
            };
            self.share = plat::share(&s);
        }
        ui.next_size(0, 44);
        if ui.button(ID_PRINT, t("Drucken")) & EV_ACT != 0 {
            self.prints += 1;
            let px = test_picture();
            let src = plat::PrintSrc { px: &px, w: PRINT_W, h: PRINT_H, stride: PRINT_W * 4, fmt: plat::PRINT_XRGB };
            let mut job = plat::PrintJob::new(Some(src), t("JFLib Galerie"));
            job.flags = plat::PJF_CENTER;
            job.margin = 1500; // 15 mm
            self.print = plat::print(&mut job);
        }
        if let Some(r) = plat::share_poll() {
            self.share = r;
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
        let shared = match self.share {
            _ if self.shares == 0 => t("noch nicht"),
            SHARE_PENDING => t("offen"),
            SHARE_DONE => t("geteilt"),
            SHARE_CANCELLED => t("abgebrochen"),
            SHARE_FAILED => t("fehlgeschlagen"),
            SHARE_NONE => t("nicht möglich"),
            _ => "?",
        };
        ui.text(TS_BODY | TX_WRAP, &tf("Teilen: {}", &[shared]));
        let printed = match self.print {
            _ if self.prints == 0 => String::from(t("noch nicht")),
            0 => String::from(t("an das System übergeben")),
            plat::ERR_UNSUPPORTED => String::from(t("nicht möglich")),
            e => format!("{} {e}", t("Fehler")),
        };
        ui.text(TS_BODY | TX_WRAP, &tf("Drucken: {}", &[&printed]));
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
             inhibited={}\nshares={}\nshare={}\nprints={}\nprint={}\n",
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
            self.shares,
            self.share,
            self.prints,
            self.print,
        );
        if text != self.written {
            plat::write_file(STATE, text.as_bytes());
            self.written = text;
        }
    }
}
