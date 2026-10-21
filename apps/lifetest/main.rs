// lifetest -- the app lifecycle of the runtime (UIC_ACTIVE, UIC_SUSPEND, UIC_SESSION_END,
// UIC_SETTINGS, UIC_THEME), the log and the settings it follows (high contrast, text scale,
// animations), for tests/life.uis and tests/settings.uis
//
//   build/win/lifetest.exe --script apps/lifetest/tests/life.uis
//
// It shows what the commands told it and the newest log line (without its time); the
// script sends the platform's events with `system ...`.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Lifecycle", flags: AF_RESIZABLE, w: 640, h: 240, ..AppDesc::NONE };

#[derive(Default)]
struct Life {
    active: bool,
    sleeping: bool,
    session_end: u32,
    settings: u32,
    themes: u32,
}

// the newest log line without its time: "INFO  app: ..."
fn last_log() -> String {
    let r = plat::log_recent(4096);
    let line = r.lines().last().unwrap_or("");
    let rest = line.split_once(' ').map_or("", |x| x.1);
    String::from(rest.split_once(' ').map_or("", |x| x.1))
}

impl App for Life {
    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "Lebenszyklus");
        let yn = |b: bool| if b { "ja" } else { "nein" };
        ui.text(TS_BODY, &format!("Aktiv: {}  Schlaf: {}  Ende: {}", yn(self.active), yn(self.sleeping), self.session_end));
        ui.text(TS_BODY, &format!("Einstellungen: {}  Theme: {}", self.settings, self.themes));
        ui.text(TS_BODY, &format!("Kontrast: {}  Text: {} %  Animationen: {}", yn(ui.high_contrast()), ui.text_scale(),
                                  if ui.anim_off() { "aus" } else { "an" }));
        ui.text(TS_BODY, &format!("Log: {}", last_log()));
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_ACTIVE => self.active = arg != 0,
            UIC_SUSPEND => self.sleeping = arg != 0,
            UIC_SESSION_END => {
                self.session_end += 1;
                plat::log_info("lifetest", "saved before the end");
            }
            UIC_SETTINGS => self.settings = arg,
            UIC_THEME => self.themes += 1,
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, Life::default(), &style::STYLE));
